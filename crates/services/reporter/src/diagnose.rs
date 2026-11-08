use std::fs;
use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::fault::{Fault, partial_snapshots, push_text, stuck_tasks};
use crate::scrub::Scrubber;
use crate::spool::{MANIFEST, STATE, Spool};

const SOCKET: &str = "/run/bande/report.sock";
const PSTORE: &str = "/var/lib/systemd/pstore";
const DESCRIPTION_LIMIT: usize = 8192;
const SECTIONS: [(&str, &str, &[&str]); 9] = [
    (
        "journal-warnings.txt",
        "journalctl",
        &[
            "-b",
            "--no-pager",
            "--output=short-iso",
            "--priority=warning",
            "-n",
            "3000",
        ],
    ),
    (
        "kernel.txt",
        "journalctl",
        &["-b", "-k", "--no-pager", "--output=short-iso", "-n", "3000"],
    ),
    (
        "processes.txt",
        "ps",
        &["-eo", "comm,pcpu,rss,stat,etimes", "--sort=-rss"],
    ),
    (
        "units-failed.txt",
        "systemctl",
        &["--failed", "--plain", "--no-legend", "--full"],
    ),
    (
        "coredumps.txt",
        "coredumpctl",
        &["list", "--no-pager", "--no-legend"],
    ),
    ("disks.txt", "lsblk", &["-o", "TYPE,SIZE,FSTYPE,ROTA,RO"]),
    (
        "disk-use.txt",
        "df",
        &["-h", "--output=fstype,size,used,pcent", "--local"],
    ),
    ("network.txt", "ip", &["-brief", "link"]),
    ("modules.txt", "lsmod", &[]),
];

fn output_of(program: &str, arguments: &[&str]) -> String {
    Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default()
}

pub fn previous_boot_tail(fault: &mut Fault, scrubber: &Scrubber, spool_root: &Path) {
    push_text(
        &mut fault.attachments,
        "previous-boot-journal.txt",
        scrubber.text(&output_of(
            "journalctl",
            &["-b", "-1", "--no-pager", "--output=short-iso", "-n", "1500"],
        )),
    );
    push_text(
        &mut fault.attachments,
        "previous-boot-kernel.txt",
        scrubber.text(&output_of(
            "journalctl",
            &[
                "-b",
                "-1",
                "-k",
                "--no-pager",
                "--output=short-iso",
                "-n",
                "1500",
            ],
        )),
    );
    for (name, text) in partial_snapshots(spool_root) {
        push_text(
            &mut fault.attachments,
            &format!("left-behind-{name}.txt"),
            scrubber.text(&text),
        );
    }
}

fn previous_boot_evidence(fault: &mut Fault, scrubber: &Scrubber, spool_root: &Path) {
    previous_boot_tail(fault, scrubber, spool_root);
    let pstore: String = fs::read_dir(PSTORE)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            format!(
                "--- {}\n{}\n",
                entry.file_name().to_string_lossy(),
                output_of(
                    "sh",
                    &["-c", &format!("tail -n 400 {}/*", entry.path().display())]
                )
            )
        })
        .collect();
    push_text(&mut fault.attachments, "pstore.txt", scrubber.text(&pstore));
}

pub fn diagnose(
    spool_root: &Path,
    config: &Config,
    peer: u32,
    description: &str,
    send: bool,
) -> io::Result<String> {
    let scrubber = Scrubber::for_this_computer();
    let spool = Spool::open(spool_root)?;
    let mut fault = Fault::new("manual");
    fault.owner = Some(peer);
    fault.signature_text = Some(fault.time.to_string());
    if !description.is_empty() {
        fault.description = Some(description.chars().take(DESCRIPTION_LIMIT).collect());
    }
    for (name, program, arguments) in SECTIONS {
        push_text(
            &mut fault.attachments,
            name,
            scrubber.text(&output_of(program, arguments)),
        );
    }
    push_text(
        &mut fault.attachments,
        "stuck-tasks.txt",
        scrubber.text(&stuck_tasks()),
    );
    previous_boot_evidence(&mut fault, &scrubber, spool_root);
    let reports: String = spool
        .all()
        .iter()
        .map(|report| {
            format!(
                "{} {} {}\n",
                report.id,
                report
                    .manifest
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("-"),
                report.occurrences
            )
        })
        .collect();
    push_text(&mut fault.attachments, "recent-reports.txt", reports);
    let id = fault.store(
        &spool,
        &Config {
            send: false,
            ..config.clone()
        },
        spool_root,
    )?;
    if send && config.server().is_some() {
        spool.queue(&id)?;
    }
    Ok(id)
}

pub fn text_of_report(spool: &Spool, id: &str) -> io::Result<String> {
    let report = spool.read(id)?;
    let mut text = serde_json::to_string_pretty(&report.manifest)?;
    text.push('\n');
    for entry in fs::read_dir(&report.path)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == MANIFEST || name == STATE || name.starts_with('.') {
            continue;
        }
        let bytes = fs::read(entry.path())?;
        text.push_str(&format!("\n--- {name}\n"));
        match std::str::from_utf8(&bytes) {
            Ok(content) => text.push_str(content),
            Err(_) => text.push_str(&format!("(binary, {} bytes)\n", bytes.len())),
        }
    }
    Ok(text)
}

pub fn ask_for_collection() -> io::Result<()> {
    let mut stream = UnixStream::connect(SOCKET)?;
    stream.write_all(b"{\"op\":\"collect\"}\n")
}
