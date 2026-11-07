use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::process::{Child, Command as Process, Stdio};
use std::sync::{Arc, Mutex};

use rustix::net::{RecvFlags, SendFlags};

use crate::{Channel, Command, Event};

pub const ENGINE: &str = "bande-media-engine";

pub struct Player {
    channel: Arc<Channel>,
    unsent: Arc<Mutex<Vec<Command>>>,
    engine: Child,
}

impl Player {
    pub fn spawn() -> io::Result<Self> {
        let (ours, theirs) = Channel::pair()?;
        let engine = Process::new(ENGINE)
            .stdin(Stdio::from(theirs.into_fd()))
            .stdout(Stdio::null())
            .spawn()?;
        Ok(Self {
            channel: Arc::new(ours),
            unsent: Arc::default(),
            engine,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.channel.as_fd()
    }

    pub fn send(&self, command: Command) {
        deliver(&self.channel, &self.unsent, command);
    }

    pub fn release_handle(&self, id: u64) -> impl FnMut() + Send + use<> {
        let channel = Arc::clone(&self.channel);
        let unsent = Arc::clone(&self.unsent);
        move || deliver(&channel, &unsent, Command::Released(id))
    }

    pub fn receive(&self) -> io::Result<Option<(Event, Vec<OwnedFd>)>> {
        flush(&self.channel, &self.unsent);
        match self.channel.receive(RecvFlags::DONTWAIT) {
            Ok(Some(received)) => Ok(Some(received)),
            Ok(None) => Err(io::ErrorKind::ConnectionAborted.into()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn deliver(channel: &Channel, unsent: &Mutex<Vec<Command>>, command: Command) {
    let Ok(mut unsent) = unsent.lock() else {
        return;
    };
    unsent.push(command);
    drain(channel, &mut unsent);
}

fn flush(channel: &Channel, unsent: &Mutex<Vec<Command>>) {
    if let Ok(mut unsent) = unsent.lock() {
        drain(channel, &mut unsent);
    }
}

fn drain(channel: &Channel, unsent: &mut Vec<Command>) {
    let sent = unsent
        .iter()
        .take_while(|command| channel.send(*command, &[], SendFlags::DONTWAIT).is_ok())
        .count();
    unsent.drain(..sent);
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.engine.kill();
        let reaped = rustix::process::Pid::from_raw(self.engine.id() as i32);
        let _ = std::thread::Builder::new()
            .name("reap".to_owned())
            .spawn(move || {
                let _ = rustix::process::waitpid(reaped, rustix::process::WaitOptions::empty());
            });
    }
}
