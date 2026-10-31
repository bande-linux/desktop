use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use unbound_bus::follow_properties;
use unbound_config::client::{INTERFACE, Properties};
use unbound_config::{BUS_NAME, DisplaySettings, OBJECT_PATH, Placement};
use unbound_inbox::{Inbox, Outbox};
use zbus::blocking::Connection;
use zbus::export::serde::de::DeserializeOwned;
use zbus::zvariant::{DeserializeDict, Type};

use crate::L10N;
use crate::about::{About, AboutCommand, follow_about};
use crate::battery::{BatteryCommand, BatteryState, follow_battery};
use crate::client::{CONFIG_SETTINGS, SettingsClient, arrangements, displays, values};
use crate::defaults::{DefaultApp, DefaultAppCommand, follow_default_apps};
use crate::failure::{sentence, unexpected};
use crate::locale::{LocaleCommand, LocaleState, follow_locale};
use crate::login::{LoginCommand, LoginItems, follow_login_items};
use crate::notifications::{AppNotifications, NotificationCommand, follow_app_notifications};
use crate::pictures::{Pair, PictureCommand, choose_file, follow_pictures};
use crate::privacy::clear_clipboard;
use crate::privilege::{Privilege, follow_privileges};
use crate::services::{
    AudioCommand, BluetoothCommand, NetworkCommand, follow_audio, follow_bluetooth, follow_network,
};
use crate::software::{SoftwareCommand, SoftwareState, follow_software};
use crate::state::{Answer, BluetoothState, Joined, NetworkState, Question, WifiState};
use crate::storage::{Storage, StorageCommand, follow_storage};
use crate::time::{TimeCommand, TimeState, follow_time};
use unbound_network::import_vpn;
use unbound_settings_panes::{SettingId, Value};

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const NOTIFICATION_CENTER: &str = "org.unbound.NotificationCenter";

#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Values(Vec<(SettingId, Value)>),
    Displays(BTreeMap<String, DisplaySettings>),
    Arrangements(Vec<Vec<Placement>>),
    Audio(Option<unbound_audio::Snapshot>),
    Wifi(Option<WifiState>),
    Network(NetworkState),
    Joined(Vec<u8>, Joined),
    Bluetooth(Option<BluetoothState>),
    Pairing(Option<Question>),
    Pictures(Vec<(String, Pair)>),
    Thumbnail(Pair, unbound_wallpaper::Picture),
    About(About),
    Storage(Storage),
    Time(Option<TimeState>),
    Login(LoginItems),
    Battery(Option<BatteryState>),
    NotificationApps(Vec<AppNotifications>),
    Software(SoftwareState),
    Locale(Option<LocaleState>),
    InputLevel(f32),
    Failed(Option<SettingId>, String),
    Refused(Vec<Privilege>),
    DefaultApps(Vec<DefaultApp>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Set(SettingId, Value),
    Display(String, DisplaySettings),
    Arrangement(Vec<Placement>),
    Audio(AudioCommand),
    Network(NetworkCommand),
    Bluetooth(BluetoothCommand),
    Answer(Answer),
    Picture(PictureCommand),
    About(AboutCommand),
    Storage(StorageCommand),
    Time(TimeCommand),
    Login(LoginCommand),
    Battery(BatteryCommand),
    Notifications(NotificationCommand),
    Software(SoftwareCommand),
    Locale(LocaleCommand),
    DefaultApp(DefaultAppCommand),
    Reveal(PathBuf),
    ImportVpn,
    ClearClipboard,
}

pub type Feed = Inbox<Update>;
pub type Updates = Outbox<Update>;

pub fn feed() -> io::Result<(Feed, Updates)> {
    let feed = Inbox::new()?;
    let updates = feed.outbox();
    Ok((feed, updates))
}

pub fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = thread::Builder::new().name(name.to_owned()).spawn(work) {
        tracing::warn!("cannot start the {name} thread: {error}");
    }
}

#[derive(DeserializeDict, Type)]
#[zvariant(
    signature = "a{sv}",
    rename_all = "PascalCase",
    crate = "zbus::zvariant"
)]
struct NotificationCenter {
    do_not_disturb: Option<bool>,
}

fn follow<T: DeserializeOwned + Type>(
    session: &Connection,
    (destination, path, interface): (&'static str, &'static str, &'static str),
    updates: Updates,
    mut read: impl FnMut(T) -> Vec<Update> + Send + 'static,
) {
    let session = session.clone();
    spawn(destination, move || {
        let followed = follow_properties(&session, destination, path, interface, |properties| {
            read(properties)
                .into_iter()
                .all(|update| updates.send(update))
        });
        if let Err(error) = followed {
            tracing::warn!("cannot follow {destination}: {error}");
        }
    });
}

fn config_updates(config: &Properties, ids: &[SettingId]) -> Vec<Update> {
    let mut read = vec![Update::Values(values(config, ids))];
    read.extend(displays(config).map(Update::Displays));
    read.extend(arrangements(config).map(Update::Arrangements));
    read
}

pub fn follow_config(session: &Connection, updates: Updates) {
    let mut config = Properties::default();
    follow(
        session,
        (BUS_NAME, OBJECT_PATH, INTERFACE),
        updates,
        move |changed| {
            config.update(changed);
            config_updates(&config, &CONFIG_SETTINGS)
        },
    );
}

pub fn follow_notifications(session: &Connection, updates: Updates) {
    follow(
        session,
        (NOTIFICATIONS, NOTIFICATIONS_PATH, NOTIFICATION_CENTER),
        updates,
        |center: NotificationCenter| {
            center
                .do_not_disturb
                .map(|on| Update::Values(vec![(SettingId::DoNotDisturb, Value::Flag(on))]))
                .into_iter()
                .collect()
        },
    );
}

fn replaces(queued: &Command, command: &Command) -> bool {
    match (queued, command) {
        (Command::Set(queued, _), Command::Set(id, _)) => queued == id,
        (Command::Display(queued, _), Command::Display(identity, _)) => queued == identity,
        (Command::Arrangement(queued), Command::Arrangement(placements)) => {
            unbound_config::same_displays(
                queued,
                &placements
                    .iter()
                    .map(|placement| placement.display.clone())
                    .collect::<Vec<_>>(),
            )
        }
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
                    Command::Display(identity, display) => {
                        (client.set_display(identity, display), Vec::new())
                    }
                    Command::Arrangement(placements) => {
                        (client.set_arrangement(placements), Vec::new())
                    }
                    _ => (Ok(()), Vec::new()),
                };
                if let Err(error) = result {
                    updates.send(Update::Failed(
                        restore.first().copied(),
                        sentence(&error, "service-desktop"),
                    ));
                    updates.send(Update::Values(client.read(&restore)));
                    if let Ok(config) = client.properties() {
                        if let Some(displays) = displays(&config) {
                            updates.send(Update::Displays(displays));
                        }
                        if let Some(arrangements) = arrangements(&config) {
                            updates.send(Update::Arrangements(arrangements));
                        }
                    }
                }
            }
        }
    });
    sender
}

pub struct Services {
    connection: Connection,
    updates: Updates,
    session: Sender<Command>,
    audio: Sender<AudioCommand>,
    network: Option<Sender<NetworkCommand>>,
    bluetooth: Option<(Sender<BluetoothCommand>, Sender<Answer>)>,
    pictures: Sender<PictureCommand>,
    about: Option<Sender<AboutCommand>>,
    storage: Sender<StorageCommand>,
    time: Option<Sender<TimeCommand>>,
    login: Sender<LoginCommand>,
    battery: Option<Sender<BatteryCommand>>,
    notifications: Sender<NotificationCommand>,
    software: Option<Sender<SoftwareCommand>>,
    locale: Option<Sender<LocaleCommand>>,
    default_apps: Sender<DefaultAppCommand>,
}

impl Services {
    pub fn start(session: &Connection, system: Option<&Connection>, updates: &Updates) -> Self {
        follow_config(session, updates.clone());
        follow_notifications(session, updates.clone());
        if let Some(system) = system {
            follow_privileges(system, updates.clone());
        }
        Self {
            connection: session.clone(),
            updates: updates.clone(),
            session: worker(session, updates.clone()),
            audio: follow_audio(updates.clone()),
            network: system.map(|system| follow_network(system, updates.clone())),
            bluetooth: system.map(|system| follow_bluetooth(system, updates.clone())),
            pictures: follow_pictures(session, unbound_xdg::data_dirs(), updates.clone()),
            about: system.map(|system| follow_about(system, updates.clone())),
            storage: follow_storage(updates.clone()),
            time: system.map(|system| follow_time(system, updates.clone())),
            login: follow_login_items(updates.clone()),
            battery: system.map(|system| follow_battery(system, updates.clone())),
            notifications: follow_app_notifications(session, updates.clone()),
            software: system.map(|system| follow_software(system, updates.clone())),
            locale: system.map(|system| follow_locale(system, updates.clone())),
            default_apps: follow_default_apps(updates.clone()),
        }
    }

    pub fn send(&self, command: Command) {
        let sent = match command {
            Command::Set(..) | Command::Display(..) | Command::Arrangement(..) => {
                self.session.send(command).is_ok()
            }

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
            Command::About(command) => self
                .about
                .as_ref()
                .is_some_and(|about| about.send(command).is_ok()),
            Command::Storage(command) => self.storage.send(command).is_ok(),
            Command::Time(command) => self
                .time
                .as_ref()
                .is_some_and(|time| time.send(command).is_ok()),
            Command::Login(command) => self.login.send(command).is_ok(),
            Command::DefaultApp(command) => self.default_apps.send(command).is_ok(),
            Command::Notifications(command) => self.notifications.send(command).is_ok(),
            Command::Locale(command) => self
                .locale
                .as_ref()
                .is_some_and(|locale| locale.send(command).is_ok()),
            Command::Software(command) => self
                .software
                .as_ref()
                .is_some_and(|software| software.send(command).is_ok()),
            Command::Battery(command) => self
                .battery
                .as_ref()
                .is_some_and(|battery| battery.send(command).is_ok()),
            Command::Reveal(_) => false,
            Command::ClearClipboard => {
                let (session, updates) = (self.connection.clone(), self.updates.clone());
                spawn("clear-clipboard", move || {
                    if let Err(error) = clear_clipboard(&session) {
                        updates.send(unexpected(error));
                    }
                });
                true
            }
            Command::ImportVpn => {
                let (session, updates) = (self.connection.clone(), self.updates.clone());
                spawn("import-vpn", move || {
                    let chosen = choose_file(
                        &session,
                        &L10N.tr("import-vpn-title"),
                        (
                            L10N.tr("vpn-files"),
                            vec![(0, "*.conf".to_owned()), (0, "*.ovpn".to_owned())],
                        ),
                    );
                    let imported = match chosen {
                        Ok(Some(file)) => import_vpn(&file).map_err(|error| error.to_string()),
                        Ok(None) => Ok(()),
                        Err(error) => Err(error.to_string()),
                    };
                    if let Err(error) = imported {
                        updates.send(Update::Failed(None, error));
                    }
                });
                true
            }
        };
        if !sent {
            tracing::warn!("a settings service is not running");
        }
    }
}
