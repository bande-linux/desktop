use std::fs;
use std::io;
use std::process::{Command, ExitCode, Stdio};

const DROP_IN_FOLDER: &str = "/etc/default/grub.d";
const DROP_IN: &str = "/etc/default/grub.d/bande-splash.cfg";
const SPLASH: &str = "GRUB_CMDLINE_LINUX_DEFAULT=\"$GRUB_CMDLINE_LINUX_DEFAULT splash\"\n";
const UPDATE_GRUB: &str = "/usr/sbin/update-grub";

fn fail(message: &str) -> ExitCode {
    eprintln!("{message}");
    ExitCode::from(2)
}

fn update_grub() -> ExitCode {
    let status = Command::new(UPDATE_GRUB)
        .env_clear()
        .env("LC_ALL", "C")
        .env("PATH", "/usr/sbin:/usr/bin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => fail(&format!("cannot run update-grub: {error}")),
    }
}

fn show(on: bool) -> io::Result<()> {
    if on {
        fs::create_dir_all(DROP_IN_FOLDER)?;
        return fs::write(DROP_IN, SPLASH);
    }
    match fs::remove_file(DROP_IN) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn main() -> ExitCode {
    if std::env::var_os("PKEXEC_UID").is_none() {
        return fail("run this through pkexec");
    }
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let on = match arguments.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["on"] => true,
        ["off"] => false,
        _ => return fail("usage: bande-boot-screen-helper on|off"),
    };
    if let Err(error) = show(on) {
        return fail(&format!("cannot change {DROP_IN}: {error}"));
    }
    update_grub()
}
