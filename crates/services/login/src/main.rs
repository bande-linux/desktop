mod account;
mod daemon;
mod journal;
mod signals;
mod vt;
mod worker;

use std::io;
use std::process::ExitCode;

use rustix::process::{DumpableBehavior, set_dumpable_behavior};

const GREETER_SOCKET: &str = "/run/bande-login/greeter.sock";

fn checked(result: libc::c_int) -> io::Result<libc::c_int> {
    match result {
        -1 => Err(io::Error::last_os_error()),
        result => Ok(result),
    }
}

fn main() -> ExitCode {
    bande_log::init("bande-login");
    if let Err(error) = set_dumpable_behavior(DumpableBehavior::NotDumpable) {
        tracing::error!("cannot keep passwords out of core dumps: {error}");
        return ExitCode::FAILURE;
    }
    let mut arguments = std::env::args_os().skip(1);
    match (arguments.next(), arguments.next()) {
        (None, _) => daemon::run(),
        (Some(argument), None) if argument == "--worker" => worker::run(),
        (Some(argument), None) if argument == "--console" => give_to_console(),
        _ => {
            tracing::error!("usage: bande-login [--console]");
            ExitCode::FAILURE
        }
    }
}

fn give_to_console() -> ExitCode {
    match vt::Terminal::open(vt::GREETER_VT).and_then(|terminal| terminal.give_to_console()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("cannot give tty{} to the console: {error}", vt::GREETER_VT);
            ExitCode::FAILURE
        }
    }
}
