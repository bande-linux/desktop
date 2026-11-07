use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, ExitCode, Stdio};

const DNSSD_FOLDER: &str = "/etc/systemd/dnssd";
const DNSSD_FILE: &str = "/etc/systemd/dnssd/bande-file-sharing.dnssd";
const DNSSD: &str = "[Service]\nName=%H\nType=_smb._tcp\nPort=445\n";
const SHARE_GROUP: &str = "sambashare";
const SAMBA_CONFIG: &str = "/etc/samba/bande-smb.conf";
const SAMBA_SETTINGS: &str = "[global]\nserver role = standalone server\nmap to guest = never\nserver min protocol = SMB2\nload printers = no\ndisable spoolss = yes\nusershare max shares = 100\nusershare allow guests = no\nusershare owner only = yes\n";
const USERSHARES: &str = "/var/lib/samba/usershares";
const SMBD_DROP_IN_FOLDER: &str = "/etc/systemd/system/smbd.service.d";
const SMBD_DROP_IN: &str = "/etc/systemd/system/smbd.service.d/bande.conf";
const SMBD_START: &str = "[Service]\nExecStart=\nExecStart=/usr/sbin/smbd --foreground --no-process-group -s /etc/samba/bande-smb.conf\n";

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(2)
}

fn run(command: &mut Command) -> ExitCode {
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/sbin:/usr/bin");
    match command.stdout(Stdio::null()).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => fail(&format!("cannot run the tool: {error}")),
    }
}

fn systemctl(arguments: &[&str]) -> ExitCode {
    run(Command::new("/usr/bin/systemctl").args(arguments))
}

fn remove_if_present(path: &str) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn write_samba_setup() -> std::io::Result<()> {
    fs::create_dir_all(DNSSD_FOLDER)?;
    fs::write(DNSSD_FILE, DNSSD)?;
    fs::write(SAMBA_CONFIG, SAMBA_SETTINGS)?;
    fs::set_permissions(USERSHARES, fs::Permissions::from_mode(0o1775))?;
    fs::create_dir_all(SMBD_DROP_IN_FOLDER)?;
    fs::write(SMBD_DROP_IN, SMBD_START)
}

fn file_sharing(on: bool) -> ExitCode {
    let changed = match on {
        true => write_samba_setup(),
        false => remove_if_present(DNSSD_FILE),
    };
    if let Err(error) = changed {
        return fail(&format!("cannot change the setup: {error}"));
    }
    systemctl(&["try-reload-or-restart", "systemd-resolved.service"]);
    match on {
        true => {
            systemctl(&["daemon-reload"]);
            systemctl(&["disable", "--now", "nmbd.service"]);
            systemctl(&["enable", "smbd.service"]);
            systemctl(&["restart", "smbd.service"])
        }
        false => systemctl(&["disable", "--now", "smbd.service", "nmbd.service"]),
    }
}

fn remote_login(on: bool) -> ExitCode {
    match on {
        true => systemctl(&["enable", "--now", "ssh.service"]),
        false => systemctl(&["disable", "--now", "ssh.service", "ssh.socket"]),
    }
}

fn user_name(uid: u32) -> Option<String> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        let name = fields.next()?;
        (fields.nth(1)?.parse::<u32>().ok()? == uid).then(|| name.to_owned())
    })
}

fn set_password(uid: u32) -> ExitCode {
    let Some(user) = user_name(uid) else {
        return fail("no such user");
    };
    let mut password = String::new();
    if std::io::stdin()
        .take(4096)
        .read_to_string(&mut password)
        .is_err()
    {
        return fail("cannot read the password");
    }
    let password = password.trim_end_matches('\n');
    if password.is_empty() || password.contains('\n') {
        return fail("bad password");
    }
    let child = Command::new("/usr/bin/smbpasswd")
        .args(["-c", SAMBA_CONFIG, "-s", "-a", &user])
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return fail(&format!("cannot run smbpasswd: {error}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = writeln!(stdin, "{password}\n{password}");
    }
    match child.wait() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        _ => ExitCode::FAILURE,
    }
}

fn share_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && !name.starts_with('-')
        && !name
            .chars()
            .any(|c| c.is_control() || "%<>*?|/\\+=;:\",".contains(c))
}

fn as_user(uid: u32, arguments: &[&str]) -> ExitCode {
    let uid = uid.to_string();
    let mut command = Command::new("/usr/bin/setpriv");
    command
        .args([
            "--reuid",
            &uid,
            "--regid",
            &uid,
            "--groups",
            SHARE_GROUP,
            "--",
        ])
        .arg("/usr/bin/net")
        .args(["-s", SAMBA_CONFIG, "usershare"])
        .args(arguments);
    run(&mut command)
}

fn main() -> ExitCode {
    let Some(uid) = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|uid| uid.parse().ok())
    else {
        return fail("run this through pkexec");
    };
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["file-sharing", "on"] => file_sharing(true),
        ["file-sharing", "off"] => file_sharing(false),
        ["remote-login", "on"] => remote_login(true),
        ["remote-login", "off"] => remote_login(false),
        ["password"] => set_password(uid),
        ["add", name, path] if share_name(name) && path.starts_with('/') => {
            as_user(uid, &["add", name, path, "", "Everyone:F", "guest_ok=n"])
        }
        ["remove", name] if share_name(name) => as_user(uid, &["delete", name]),
        _ => fail("bad arguments"),
    }
}
