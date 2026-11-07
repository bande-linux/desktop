use std::fs;
use std::io::{self, BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Serialize;

use crate::disk::{self, REPOSITORIES};
use crate::restic::{Exit, Progress, Repository, Stopper};
use crate::source::{self, EXCLUDED};
use crate::store::SecretFile;

const KEPT_FROM_THE_NEW_SYSTEM: [&str; 3] = ["/etc/fstab", "/etc/crypttab", "/etc/kernel/cmdline"];
const OPEN_TO_ALL: u32 = 0o1777;
const SYSTEM_FOLDER: u32 = 0o755;
const WRONG_PASSWORD: u8 = 2;
const SOME_FILES_LOST: u8 = 3;
const FAILED: u8 = 1;
const USAGE: &str = "usage: bande-backup-restore list | restore --uuid UUID --filesystem TYPE --machine ID --target DIR [--keep PATH]... [--time UNIX-SECONDS]\n";
const MOUNT_OPTIONS: &str = "ro,nosuid,nodev,noexec,noatime";

#[derive(Serialize)]
struct Found {
    uuid: String,
    filesystem: String,
    machine: String,
    name: String,
}

fn blkid_type(device: &Path) -> Option<String> {
    let output = Command::new("blkid")
        .args(["-o", "value", "-s", "TYPE"])
        .arg(device)
        .output()
        .ok()?;
    let kind = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!kind.is_empty()).then_some(kind)
}

fn with_mount<T>(uuid: &str, filesystem: &str, work: impl FnOnce(&Path) -> T) -> io::Result<T> {
    disk::prepare_run_dir()?;
    let mount_point = Path::new(disk::RUN_DIR).join("recovery");
    fs::create_dir_all(&mount_point)?;
    let status = Command::new("mount")
        .args(["-t", disk::mount_type(filesystem), "-o", MOUNT_OPTIONS])
        .arg(disk::device(uuid))
        .arg(&mount_point)
        .stderr(std::process::Stdio::null())
        .status()?;
    if !status.success() {
        return Err(io::Error::other(format!("mount: {status}")));
    }
    let result = work(&mount_point);
    if let Err(error) = Command::new("umount").arg(&mount_point).status() {
        tracing::warn!("cannot unmount the backup disk: {error}");
    }
    Ok(result)
}

fn list() -> io::Result<Vec<Found>> {
    let mut found = Vec::new();
    let Ok(devices) = fs::read_dir("/dev/disk/by-uuid") else {
        return Ok(found);
    };
    for device in devices.flatten() {
        let uuid = device.file_name().to_string_lossy().into_owned();
        let Some(filesystem) = blkid_type(&device.path()) else {
            continue;
        };
        let scan = with_mount(&uuid, &filesystem, |mount_point| {
            let mut here = Vec::new();
            let Ok(entries) = fs::read_dir(mount_point.join(REPOSITORIES)) else {
                return here;
            };
            for entry in entries.flatten() {
                if disk::has_repository(&entry.path()) {
                    let machine = entry.file_name().to_string_lossy().into_owned();
                    let label = mount_point
                        .join(REPOSITORIES)
                        .join(format!("{machine}.name"));
                    let name = fs::read_to_string(label).unwrap_or_else(|_| machine.clone());
                    here.push(Found {
                        uuid: uuid.clone(),
                        filesystem: filesystem.clone(),
                        machine,
                        name: name.trim().to_owned(),
                    });
                }
            }
            here
        });
        if let Ok(here) = scan {
            found.extend(here);
        }
    }
    Ok(found)
}

struct RestoreRequest {
    uuid: String,
    filesystem: String,
    machine: String,
    target: PathBuf,
    keep: Vec<String>,
    time: Option<u64>,
}

fn parse(args: &[String]) -> Option<RestoreRequest> {
    let mut request = RestoreRequest {
        uuid: String::new(),
        filesystem: String::new(),
        machine: String::new(),
        target: PathBuf::new(),
        keep: KEPT_FROM_THE_NEW_SYSTEM.map(str::to_owned).to_vec(),
        time: None,
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args.next()?.clone();
        match flag.as_str() {
            "--uuid" => request.uuid = value,
            "--filesystem" => request.filesystem = value,
            "--machine" => request.machine = value,
            "--target" => request.target = PathBuf::from(value),
            "--keep" => request.keep.push(value),
            "--time" => request.time = Some(value.parse().ok()?),
            _ => return None,
        }
    }
    let complete = !request.uuid.is_empty()
        && !request.filesystem.is_empty()
        && !request.machine.is_empty()
        && request.target.is_absolute()
        && request.target.parent().is_some();
    complete.then_some(request)
}

fn mounts_below(target: &Path) -> Vec<String> {
    let Ok(mounts) = fs::read_to_string("/proc/self/mounts") else {
        return Vec::new();
    };
    let sources = source::sources_in(&mounts);
    let mut below = Vec::new();
    for line in mounts.lines() {
        let Some(point) = line.split(' ').nth(1) else {
            continue;
        };
        let point = Path::new(point);
        if let Ok(rest) = point.strip_prefix(target)
            && !rest.as_os_str().is_empty()
            && !sources.iter().any(|source| source == point)
        {
            below.push(format!("/{}", rest.display()));
        }
    }
    below
}

fn snapshot_at(repository: &Repository, machine: &str, time: Option<u64>) -> io::Result<String> {
    let snapshots = repository.snapshots(&Stopper::default(), machine)?;
    let chosen = match time {
        Some(time) => snapshots
            .iter()
            .rfind(|snapshot| snapshot.seconds() <= time),
        None => snapshots.last(),
    };
    chosen
        .map(|snapshot| snapshot.id.clone())
        .ok_or_else(|| io::Error::other("no backups on this disk"))
}

fn stash(target: &Path, keep: &[String]) -> Vec<(PathBuf, Vec<u8>)> {
    keep.iter()
        .filter_map(|path| {
            let inside = target.join(path.trim_start_matches('/'));
            fs::read(&inside).ok().map(|bytes| (inside, bytes))
        })
        .collect()
}

fn recreate_excluded_folders(target: &Path) -> io::Result<()> {
    for path in EXCLUDED {
        if path.contains('*') || path.contains("swap") || path.starts_with("/home") {
            continue;
        }
        let inside = target.join(path.trim_start_matches('/'));
        fs::create_dir_all(&inside)?;
        let mode = if path == "/tmp" || path == "/var/tmp" {
            OPEN_TO_ALL
        } else {
            SYSTEM_FOLDER
        };
        fs::set_permissions(&inside, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn rebuild_boot(target: &Path) {
    let pseudo = ["proc", "sys", "dev", "run"];
    let mut bound = Vec::new();
    for name in pseudo {
        let inside = target.join(name);
        let ok = Command::new("mount")
            .args(["--rbind", &format!("/{name}")])
            .arg(&inside)
            .status()
            .is_ok_and(|status| status.success());
        if ok {
            bound.push(inside);
        }
    }
    for step in [
        "command -v update-initramfs >/dev/null && update-initramfs -u -k all",
        "command -v update-grub >/dev/null && update-grub",
    ] {
        let done = Command::new("chroot")
            .arg(target)
            .args(["/bin/sh", "-c", step])
            .status();
        match done {
            Ok(status) if status.success() => {}
            Ok(status) => tracing::warn!("{step}: {status}"),
            Err(error) => tracing::warn!("{step}: {error}"),
        }
    }
    for inside in bound.iter().rev() {
        if let Err(error) = Command::new("umount").arg("-lR").arg(inside).status() {
            tracing::warn!("cannot unmount {}: {error}", inside.display());
        }
    }
}

fn restore(request: &RestoreRequest, password: &str) -> Result<Exit, io::Error> {
    let secret = SecretFile::new(password)?;
    with_mount(&request.uuid, &request.filesystem, |mount_point| {
        let repository = Repository::new(
            mount_point.join(REPOSITORIES).join(&request.machine),
            secret.path(),
        );
        let stopper = Stopper::default();
        match repository.opens(&stopper)? {
            Exit::Done => {}
            other => return Ok(other),
        }
        let snapshot = snapshot_at(&repository, &request.machine, request.time)?;
        let kept = stash(&request.target, &request.keep);
        let mut args = vec![
            "restore".to_owned(),
            snapshot,
            "--json".to_owned(),
            "--no-cache".to_owned(),
            "--no-lock".to_owned(),
            "--delete".to_owned(),
            "--verify".to_owned(),
            "--target".to_owned(),
            request.target.to_string_lossy().into_owned(),
        ];
        for path in EXCLUDED
            .iter()
            .map(|path| (*path).to_owned())
            .chain(mounts_below(&request.target))
            .chain(request.keep.iter().cloned())
        {
            args.push("--exclude".to_owned());
            args.push(path);
        }
        let finished = repository.follow_progress(&stopper, &args, |progress: Progress| {
            println!("{{\"percent\":{:.4}}}", progress.percent_done);
            if let Err(error) = io::stdout().flush() {
                tracing::warn!("cannot report progress: {error}");
            }
        })?;
        if finished.exit != Exit::Done {
            tracing::error!("restic restore: {}", finished.output);
            eprintln!("{}", finished.output);
        }
        for (path, bytes) in &kept {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, bytes)?;
        }
        recreate_excluded_folders(&request.target)?;
        rebuild_boot(&request.target);
        Ok(finished.exit)
    })?
}

pub fn main(args: Vec<String>) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("list") => {
            match list().and_then(|found| serde_json::to_string(&found).map_err(io::Error::other)) {
                Ok(found) => {
                    println!("{found}");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(FAILED)
                }
            }
        }
        Some("restore") => {
            let Some(request) = parse(&args[1..]) else {
                eprint!("{USAGE}");
                return ExitCode::from(FAILED);
            };
            let mut password = String::new();
            if let Err(error) = io::stdin().lock().read_line(&mut password) {
                eprintln!("{error}");
                return ExitCode::from(FAILED);
            }
            match restore(&request, password.trim_end_matches('\n')) {
                Ok(Exit::Done) => ExitCode::SUCCESS,
                Ok(Exit::WrongPassword) => ExitCode::from(WRONG_PASSWORD),
                Ok(Exit::Failed | Exit::SomeFilesUnreadable) => ExitCode::from(SOME_FILES_LOST),
                Ok(_) => ExitCode::from(FAILED),
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::from(FAILED)
                }
            }
        }
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(FAILED)
        }
    }
}
