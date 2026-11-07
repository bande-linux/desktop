use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, chown};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use bande_volumes::{mounted, system_mounts_root};

const CIFS: &str = "/usr/sbin/mount.cifs";
const DAVFS: &str = "/usr/sbin/mount.davfs";
const UMOUNT: &str = "/usr/bin/umount";
const DAVFS_CONF: &str = "/usr/share/bande/bande-davfs2.conf";
const SOFT_MOUNT: &str = "soft,echo_interval=5,actimeo=1";
const UNMOUNTABLE: [&str; 3] = ["cifs", "smb3", "fuse"];

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(2)
}

fn plain(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('-') && !text.chars().any(|c| c.is_control() || c == ',')
}

fn volume_name(name: &str) -> bool {
    plain(name) && name != "." && name != ".." && !name.contains('/')
}

fn host(text: &str) -> bool {
    !text.starts_with('-')
        && !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
}

fn primary_gid(uid: u32) -> Option<u32> {
    fs::metadata(format!("/run/user/{uid}"))
        .ok()
        .filter(|metadata| metadata.uid() == uid)
        .map(|metadata| metadata.gid())
}

fn point_for(uid: u32, gid: u32, name: &str) -> std::io::Result<PathBuf> {
    let root = system_mounts_root(uid);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(root.parent().unwrap_or(Path::new("/run")))?;
    match fs::DirBuilder::new().mode(0o750).create(&root) {
        Ok(()) => chown(&root, Some(0), Some(gid))?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let point = root.join(name);
    fs::DirBuilder::new().mode(0o755).create(&point)?;
    Ok(point)
}

fn run(mut command: Command, password: Option<&str>) -> ExitCode {
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/sbin:/usr/bin");
    if let Some(password) = password {
        command.env("PASSWD", password);
    }
    command.stdout(Stdio::null());
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => fail(&format!("cannot start the mount tool: {error}")),
    }
}

fn mount_cifs(uid: u32, gid: u32, unc: &str, point: &Path, user: &str, port: &str) -> ExitCode {
    let password = (!user.is_empty()).then(|| {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        line.trim_end_matches('\n').to_owned()
    });
    let mut options = format!("{SOFT_MOUNT},nosuid,nodev,uid={uid},gid={gid},forceuid,forcegid");
    if user.is_empty() {
        options.push_str(",guest");
    } else {
        let (domain, name) = user.split_once('\\').unwrap_or(("", user));
        options.push_str(&format!(",username={name}"));
        if !domain.is_empty() {
            options.push_str(&format!(",domain={domain}"));
        }
    }
    if !port.is_empty() {
        options.push_str(&format!(",port={port}"));
    }
    let mut command = Command::new(CIFS);
    command.arg(unc).arg(point).arg("-o").arg(options);
    command.stdin(Stdio::null());
    run(command, password.as_deref())
}

fn mount_davfs(uid: u32, gid: u32, url: &str, point: &Path) -> ExitCode {
    let mut command = Command::new(DAVFS);
    command.arg(url).arg(point).arg("-o").arg(format!(
        "conf={DAVFS_CONF},uid={uid},gid={gid},file_mode=0644,dir_mode=0755,nosuid,nodev"
    ));
    command.stdin(Stdio::inherit());
    run(command, None)
}

fn mount(uid: u32, arguments: &[String]) -> ExitCode {
    let [kind, source, name, user, port] = arguments else {
        return fail("bad arguments");
    };
    if !volume_name(name)
        || (!user.is_empty() && !plain(user))
        || !port.chars().all(|c| c.is_ascii_digit())
    {
        return fail("bad arguments");
    }
    let Some(gid) = primary_gid(uid) else {
        return fail("no session for this user");
    };
    let valid = match kind.as_str() {
        "cifs" => source
            .strip_prefix("//")
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(server, share)| host(server) && plain(share) && !share.contains('/')),
        "davfs" => {
            (source.starts_with("http://") || source.starts_with("https://")) && plain(source)
        }
        _ => false,
    };
    if !valid {
        return fail("bad arguments");
    }
    let point = match point_for(uid, gid, name) {
        Ok(point) => point,
        Err(error) => return fail(&format!("cannot make the mount point: {error}")),
    };
    let code = match kind.as_str() {
        "cifs" => mount_cifs(uid, gid, source, &point, user, port),
        _ => mount_davfs(uid, gid, source, &point),
    };
    if code != ExitCode::SUCCESS {
        let _ = fs::remove_dir(&point);
    }
    code
}

fn unmount_all(uid: u32, name: Option<&str>) -> ExitCode {
    let root = system_mounts_root(uid);
    let table = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    for mount in mounted(&table) {
        let wanted = mount.point.parent() == Some(root.as_path())
            && UNMOUNTABLE.contains(&mount.fs_type.as_str())
            && name.is_none_or(|name| mount.point.file_name() == Some(name.as_ref()));
        if !wanted {
            continue;
        }
        let mut command = Command::new(UMOUNT);
        match mount.fs_type.as_str() {
            "fuse" => command.arg("-l"),
            _ => command.args(["-f", "-l"]),
        };
        command.arg(&mount.point).stdin(Stdio::null());
        run(command, None);
        let _ = fs::remove_dir(&mount.point);
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    if !rustix::process::geteuid().is_root() {
        return fail("run this through pkexec");
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let caller = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|uid| uid.parse::<u32>().ok());
    match (arguments.split_first(), caller) {
        (Some((command, rest)), Some(uid)) if command == "mount" => mount(uid, rest),
        (Some((command, [name])), Some(uid)) if command == "unmount" && volume_name(name) => {
            unmount_all(uid, Some(name))
        }
        (Some((command, [uid])), None) if command == "cleanup" => match uid.parse() {
            Ok(uid) => unmount_all(uid, None),
            Err(_) => fail("bad arguments"),
        },
        _ => fail("bad arguments"),
    }
}
