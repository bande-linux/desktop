use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use bande_settings::{SPLASH_DROP_IN, switch_splash};

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
    let text = match fs::read_to_string(SPLASH_DROP_IN) {
        Err(error) if error.kind() == io::ErrorKind::NotFound && !on => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        read => read?,
    };
    let switched = switch_splash(&text, on);
    match switched == text {
        true => Ok(()),
        false => bande_xdg::replace_file(Path::new(SPLASH_DROP_IN), switched),
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
        return fail(&format!("cannot change {SPLASH_DROP_IN}: {error}"));
    }
    update_grub()
}
