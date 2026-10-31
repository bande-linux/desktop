use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::thread;

use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, waitid};
use unbound_xdg::{Failed, app_scope, ran_out_of_memory, start_scope};
use zbus::blocking::Connection;

use crate::APP_ID;

pub fn follow(bus: Connection, shell: u32) -> io::Result<UnixStream> {
    let (ending, mut sender) = UnixStream::pair()?;
    ending.set_nonblocking(true)?;
    thread::Builder::new()
        .name("terminal-tab".to_owned())
        .spawn(move || {
            let out_of_memory = watch(&bus, shell);
            if let Err(error) = sender.write_all(&[u8::from(out_of_memory)]) {
                tracing::debug!("the tab of {shell} is gone: {error}");
            }
        })?;
    Ok(ending)
}

fn watch(bus: &Connection, shell: u32) -> bool {
    let unit = app_scope(APP_ID, format_args!("tab{shell}"));
    if let Err(error) = start_scope(bus, &unit, shell, Failed::Keep) {
        tracing::debug!("the shell {shell} runs outside its own scope: {error}");
        return false;
    }
    let killed = i32::try_from(shell)
        .ok()
        .and_then(Pid::from_raw)
        .and_then(|pid| {
            waitid(
                WaitId::Pid(pid),
                WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
            )
            .ok()
        })
        .flatten()
        .is_some_and(|status| status.terminating_signal() == Some(Signal::KILL.as_raw()));
    killed && ran_out_of_memory(bus, &unit)
}
