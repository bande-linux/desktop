use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use unbound_config::{BUS_NAME, OBJECT_PATH, Output};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::client::{CONFIG_SETTINGS, SettingsClient};
use crate::pictures::{Pair, PictureCommand, follow_pictures};
use crate::services::{
    AudioCommand, BluetoothCommand, NetworkCommand, follow_audio, follow_bluetooth, follow_network,
};
use crate::state::{Answer, BluetoothState, Joined, Question, WifiState};
use unbound_settings_panes::{SettingId, Value};

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";

#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Values(Vec<(SettingId, Value)>),
    Outputs(BTreeMap<String, Output>),
    Audio(Option<unbound_audio::Snapshot>),
    Wifi(Option<WifiState>),
    Joined(Vec<u8>, Joined),
    Bluetooth(Option<BluetoothState>),
    Pairing(Option<Question>),
    Pictures(Vec<(String, Pair)>),
    Thumbnail(Pair, unbound_wallpaper::Picture),
    InputLevel(f32),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Set(SettingId, Value),
    Output(String, Output),
    Audio(AudioCommand),
    Network(NetworkCommand),
    Bluetooth(BluetoothCommand),
    Answer(Answer),
    Picture(PictureCommand),
}

pub struct Feed {
    receiver: Receiver<Update>,
    wake: UnixStream,
}

#[derive(Clone)]
pub struct Updates {
    sender: Sender<Update>,
    wake: Arc<UnixStream>,
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
            wake: Arc::new(writer),
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

pub fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = thread::Builder::new().name(name.to_owned()).spawn(work) {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}

fn follow_properties(
    session: &Connection,
    destination: &'static str,
    path: &'static str,
    updates: Updates,
    read: impl Fn(&SettingsClient) -> Vec<Update> + Send + 'static,
) {
    let session = session.clone();
    spawn(destination, move || {
        let follow = || -> zbus::Result<()> {
            let client = SettingsClient::connect(&session)?;
            let changes = PropertiesProxy::builder(&session)
                .destination(destination)?
                .path(path)?
                .build()?
                .receive_properties_changed()?;
            let publish = || read(&client).into_iter().all(|update| updates.send(update));
            if !publish() {
                return Ok(());
            }
            for _ in changes {
                if !publish() {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow {destination}: {error}");
        }
    });
}

pub fn follow_config(session: &Connection, updates: Updates) {
    follow_properties(session, BUS_NAME, OBJECT_PATH, updates, |client| {
        let mut read = vec![Update::Values(client.read(&CONFIG_SETTINGS))];
        match client.outputs() {
            Ok(outputs) => read.push(Update::Outputs(outputs)),
            Err(error) => tracing::warn!("cannot read the display settings: {error}"),
        }
        read
    });
}

pub fn follow_notifications(session: &Connection, updates: Updates) {
    follow_properties(
        session,
        NOTIFICATIONS,
        NOTIFICATIONS_PATH,
        updates,
        |client| vec![Update::Values(client.read(&[SettingId::DoNotDisturb]))],
    );
}

fn replaces(queued: &Command, command: &Command) -> bool {
    match (queued, command) {
        (Command::Set(queued, _), Command::Set(id, _)) => queued == id,
        (Command::Output(queued, _), Command::Output(connector, _)) => queued == connector,
        _ => false,
    }
}

pub fn worker(session: &Connection, updates: Updates) -> Sender<Command> {
    let (sender, receiver) = mpsc::channel::<Command>();
    let session = session.clone();
    spawn("settings-worker", move || {
        let client = match SettingsClient::connect(&session) {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!("cannot reach the session services: {error}");
                return;
            }
        };
        while let Ok(first) = receiver.recv() {
            let mut commands = vec![first];
            for command in receiver.try_iter() {
                commands.retain(|queued| !replaces(queued, &command));
                commands.push(command);
            }
            for command in commands {
                let (result, restore) = match &command {
                    Command::Set(id, value) => (client.set(*id, value), vec![*id]),
                    Command::Output(connector, output) => {
                        (client.set_output(connector, output), Vec::new())
                    }
                    _ => (Ok(()), Vec::new()),
                };
                if let Err(error) = result {
                    updates.send(Update::Failed(error.to_string()));
                    updates.send(Update::Values(client.read(&restore)));
                    if let Ok(outputs) = client.outputs() {
                        updates.send(Update::Outputs(outputs));
                    }
                }
            }
        }
    });
    sender
}

pub struct Services {
    session: Sender<Command>,
    audio: Sender<AudioCommand>,
    network: Option<Sender<NetworkCommand>>,
    bluetooth: Option<(Sender<BluetoothCommand>, Sender<Answer>)>,
    pictures: Sender<PictureCommand>,
}

impl Services {
    pub fn start(session: &Connection, system: Option<&Connection>, updates: &Updates) -> Self {
        follow_config(session, updates.clone());
        follow_notifications(session, updates.clone());
        Self {
            session: worker(session, updates.clone()),
            audio: follow_audio(updates.clone()),
            network: system.map(|system| follow_network(system, updates.clone())),
            bluetooth: system.map(|system| follow_bluetooth(system, updates.clone())),
            pictures: follow_pictures(session, unbound_xdg::data_dirs(), updates.clone()),
        }
    }

    pub fn send(&self, command: Command) {
        let sent = match command {
            Command::Set(..) | Command::Output(..) => self.session.send(command).is_ok(),
            Command::Audio(command) => self.audio.send(command).is_ok(),
            Command::Network(command) => self
                .network
                .as_ref()
                .is_some_and(|network| network.send(command).is_ok()),
            Command::Bluetooth(command) => self
                .bluetooth
                .as_ref()
                .is_some_and(|(bluetooth, _)| bluetooth.send(command).is_ok()),
            Command::Answer(answer) => self
                .bluetooth
                .as_ref()
                .is_some_and(|(_, answers)| answers.send(answer).is_ok()),
            Command::Picture(command) => self.pictures.send(command).is_ok(),
        };
        if !sent {
            tracing::warn!("a settings service is not running");
        }
    }
}
