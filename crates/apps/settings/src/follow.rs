use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use unbound_config::{BUS_NAME, OBJECT_PATH};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::client::SettingsClient;
use crate::registry::{SettingId, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Values(Vec<(SettingId, Value)>),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Set(SettingId, Value),
}

pub struct Feed {
    receiver: Receiver<Update>,
    wake: UnixStream,
}

#[derive(Clone)]
pub struct Updates {
    sender: Sender<Update>,
    wake: std::sync::Arc<UnixStream>,
}

impl Updates {
    pub fn send(&self, update: Update) -> bool {
        self.sender.send(update).is_ok() && (&*self.wake).write_all(&[1]).is_ok()
    }
}

pub fn feed() -> io::Result<(Feed, Updates)> {
    let (reader, writer) = UnixStream::pair()?;
    reader.set_nonblocking(true)?;
    let (sender, receiver) = mpsc::channel();
    Ok((
        Feed {
            receiver,
            wake: reader,
        },
        Updates {
            sender,
            wake: std::sync::Arc::new(writer),
        },
    ))
}

impl Feed {
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.wake.as_fd()
    }

    pub fn drain(&self) -> Vec<Update> {
        let mut bytes = [0; 64];
        while matches!((&self.wake).read(&mut bytes), Ok(read) if read > 0) {}
        self.receiver.try_iter().collect()
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = thread::Builder::new().name(name.to_owned()).spawn(work) {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}

pub fn follow_config(session: &Connection, updates: Updates) {
    let session = session.clone();
    spawn("config", move || {
        let follow = || -> zbus::Result<()> {
            let client = SettingsClient::connect(&session)?;
            let changes = PropertiesProxy::builder(&session)
                .destination(BUS_NAME)?
                .path(OBJECT_PATH)?
                .build()?
                .receive_properties_changed()?;
            if !updates.send(Update::Values(client.read_all())) {
                return Ok(());
            }
            for _ in changes {
                if !updates.send(Update::Values(client.read_all())) {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow org.unbound.Config: {error}");
        }
    });
}

fn coalesced(first: Command, rest: impl Iterator<Item = Command>) -> Vec<Command> {
    let mut commands = vec![first];
    for command in rest {
        let Command::Set(id, _) = &command;
        commands.retain(|queued| !matches!(queued, Command::Set(queued, _) if queued == id));
        commands.push(command);
    }
    commands
}

pub fn worker(session: &Connection, updates: Updates) -> Sender<Command> {
    let (sender, receiver) = mpsc::channel::<Command>();
    let session = session.clone();
    spawn("settings-worker", move || {
        let client = match SettingsClient::connect(&session) {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!("cannot reach org.unbound.Config: {error}");
                return;
            }
        };
        while let Ok(first) = receiver.recv() {
            for command in coalesced(first, receiver.try_iter()) {
                let Command::Set(id, value) = command;
                if let Err(error) = client.set(id, &value) {
                    updates.send(Update::Failed(error.to_string()));
                    updates.send(Update::Values(client.read_all()));
                }
            }
        }
    });
    sender
}
