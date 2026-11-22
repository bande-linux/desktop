use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use bande_network::NetworkManager;
use bande_network::ifupdown::{self, Plan, Refusal};

const IFUP: &str = "/usr/sbin/ifup";
const IFDOWN: &str = "/usr/sbin/ifdown";
const IFQUERY: &str = "/usr/sbin/ifquery";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const INTERFACES: &str = "/sys/class/net";
const COMPETING_SERVICES: [&str; 1] = ["dhcpcd.service"];
const REFUSED: u8 = 3;

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::FAILURE
}

fn quiet(program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn valid_interface(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        && Path::new(INTERFACES).join(name).exists()
}

fn planned(interface: &str) -> Result<Plan, ExitCode> {
    let refuse = |refusal: Refusal| {
        println!("refused {}", refusal.code());
        ExitCode::from(REFUSED)
    };
    if let Some(service) = COMPETING_SERVICES
        .iter()
        .find(|service| quiet(SYSTEMCTL, &["is-active", "--quiet", service]))
    {
        return Err(refuse(Refusal::Service((*service).to_owned())));
    }
    let configuration = ifupdown::read()
        .map_err(|error| fail(&format!("cannot read the interfaces files: {error}")))?;
    let wireless = Path::new(INTERFACES)
        .join(interface)
        .join("wireless")
        .exists();
    ifupdown::plan(&configuration, interface, wireless).map_err(refuse)
}

fn print_plan(plan: &Plan, backups: &[PathBuf]) {
    for (index, edit) in plan.edits.iter().enumerate() {
        println!("file {}", edit.path.display());
        if let Some(backup) = backups.get(index) {
            println!("backup {}", backup.display());
        }
        for line in edit.disabled() {
            println!("line {line}");
        }
    }
}

fn backup_path(path: &Path, stamp: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sourced = path.parent() == Some(Path::new(ifupdown::INTERFACES_FOLDER));
    let hidden = if sourced { "." } else { "" };
    path.with_file_name(format!("{hidden}{name}.bande-{stamp}"))
}

fn write_backup(path: &Path, backup: &Path, original: &str) -> io::Result<()> {
    let mode = fs::metadata(path)?.permissions().mode() & 0o7777;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(backup)?;
    file.write_all(original.as_bytes())?;
    file.sync_all()
}

fn discard(backups: &[PathBuf]) {
    for backup in backups {
        if let Err(error) = fs::remove_file(backup) {
            eprintln!("cannot remove {}: {error}", backup.display());
        }
    }
}

fn restore(plan: &Plan, backups: &[PathBuf], was_up: bool) {
    for edit in &plan.edits {
        if let Err(error) = bande_xdg::replace_file(&edit.path, &edit.original) {
            eprintln!("cannot restore {}: {error}", edit.path.display());
        }
    }
    if was_up && !quiet(IFUP, &[&plan.interface]) {
        eprintln!("cannot bring {} up again", plan.interface);
    }
    discard(backups);
}

fn manage(interface: &str) -> ExitCode {
    let plan = match planned(interface) {
        Ok(plan) => plan,
        Err(code) => return code,
    };
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let mut backups = Vec::new();
    for edit in &plan.edits {
        let backup = backup_path(&edit.path, &stamp);
        if let Err(error) = write_backup(&edit.path, &backup, &edit.original) {
            discard(&backups);
            return fail(&format!(
                "cannot keep a copy of {}: {error}",
                edit.path.display()
            ));
        }
        backups.push(backup);
    }
    let was_up = quiet(IFQUERY, &["--state", interface]);
    if was_up && !quiet(IFDOWN, &[interface]) {
        discard(&backups);
        return fail(&format!("ifdown could not take {interface} down"));
    }
    for (edit, backup) in plan.edits.iter().zip(&backups) {
        let note = format!(
            "Bande Settings handed {interface} to NetworkManager on {stamp}; the original file is {}",
            backup.display()
        );
        if let Err(error) = bande_xdg::replace_file(&edit.path, edit.edited(&note)) {
            restore(&plan, &backups, was_up);
            return fail(&format!("cannot change {}: {error}", edit.path.display()));
        }
    }
    let taken = zbus::blocking::Connection::system()
        .map_err(bande_network::Error::from)
        .and_then(|system| NetworkManager::new(&system).take_over(&plan));
    match taken {
        Ok(connection) => {
            print_plan(&plan, &backups);
            println!("connection {connection}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            restore(&plan, &backups, was_up);
            fail(&format!(
                "NetworkManager did not take {interface} over: {error}"
            ))
        }
    }
}

fn main() -> ExitCode {
    if std::env::var_os("PKEXEC_UID").is_none() {
        return fail("run this through pkexec");
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["preview", interface] if valid_interface(interface) => match planned(interface) {
            Ok(plan) => {
                print_plan(&plan, &[]);
                ExitCode::SUCCESS
            }
            Err(code) => code,
        },
        ["manage", interface] if valid_interface(interface) => manage(interface),
        _ => fail("bad arguments"),
    }
}
