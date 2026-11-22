use std::ffi::CStr;
use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};

const SHARE_GROUP: &str = "sambashare";
const SAMBA_UNITS: [&str; 2] = ["smbd.service", "nmbd.service"];

struct Account {
    name: String,
    group: u32,
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(2)
}

fn run(command: &mut Command) -> ExitCode {
    command
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/sbin:/usr/bin");
    match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("{:?} ended with {status}", command.get_program());
            ExitCode::FAILURE
        }
        Err(error) => fail(&format!("cannot run {:?}: {error}", command.get_program())),
    }
}

fn systemctl(arguments: &[&str]) -> ExitCode {
    run(Command::new("/usr/bin/systemctl")
        .args(arguments)
        .stdout(Stdio::null()))
}

fn samba_units(verb: &str, now: bool) -> ExitCode {
    let mut arguments = vec![verb];
    if now {
        arguments.push("--now");
    }
    arguments.extend(SAMBA_UNITS);
    systemctl(&arguments)
}

fn file_sharing(on: bool) -> ExitCode {
    match on {
        true if samba_units("unmask", false) != ExitCode::SUCCESS => ExitCode::FAILURE,
        true => systemctl(&["enable", "--now", "smbd.service"]),
        false => systemctl(&["disable", "--now", "smbd.service"]),
    }
}

fn remote_login(on: bool) -> ExitCode {
    match on {
        true => systemctl(&["enable", "--now", "ssh.service"]),
        false => systemctl(&["disable", "--now", "ssh.service", "ssh.socket"]),
    }
}

fn account(uid: u32) -> Option<Account> {
    // SAFETY: passwd is plain C data (integers and pointers) for which all zeros is valid.
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0u8; 4096];
    loop {
        // SAFETY: buffer is writable for buffer.len() bytes and outlives entry, whose string
        // fields point into it and are read below.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut found,
            )
        };
        match status {
            0 => break,
            libc::ERANGE => buffer.resize(buffer.len() * 2, 0),
            _ => return None,
        }
    }
    if found.is_null() || entry.pw_name.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r filled entry, and its non-null pw_name is a NUL-terminated string in
    // buffer, which is still alive.
    let name = unsafe { CStr::from_ptr(entry.pw_name) };
    Some(Account {
        name: name.to_str().ok()?.to_owned(),
        group: entry.pw_gid,
    })
}

fn set_password(user: &Account) -> ExitCode {
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
        .args(["-s", "-a", &user.name])
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return fail(&format!("cannot run smbpasswd: {error}")),
    };
    if let Some(mut stdin) = child.stdin.take()
        && let Err(error) = writeln!(stdin, "{password}\n{password}")
    {
        eprintln!("cannot pass the password to smbpasswd: {error}");
    }
    match child.wait() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => fail(&format!("smbpasswd ended with {status}")),
        Err(error) => fail(&format!("cannot wait for smbpasswd: {error}")),
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

fn usershare(uid: u32, user: &Account, arguments: &[&str]) -> ExitCode {
    let mut command = Command::new("/usr/bin/setpriv");
    command
        .args([
            "--reuid",
            &uid.to_string(),
            "--regid",
            &user.group.to_string(),
        ])
        .args(["--groups", SHARE_GROUP, "--"])
        .args(["/usr/bin/net", "usershare"])
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
    let Some(user) = account(uid) else {
        return fail("no such user");
    };
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
    match words.as_slice() {
        ["file-sharing", "on"] => file_sharing(true),
        ["file-sharing", "off"] => file_sharing(false),
        ["samba-installing"] => samba_units("mask", false),
        ["samba-not-installed"] => samba_units("unmask", false),
        ["samba-installed"] => samba_units("disable", true),
        ["remote-login", "on"] => remote_login(true),
        ["remote-login", "off"] => remote_login(false),
        ["password"] => set_password(&user),
        ["list"] => usershare(uid, &user, &["info"]),
        ["add", name, path] if share_name(name) && path.starts_with('/') => usershare(
            uid,
            &user,
            &["add", name, path, "", "Everyone:F", "guest_ok=n"],
        ),
        ["remove", name] if share_name(name) => usershare(uid, &user, &["delete", name]),
        _ => fail("bad arguments"),
    }
}
