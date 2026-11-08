use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use bande_app::choose_file;
use bande_config::{BUS_NAME, DisplaySettings, OBJECT_PATH, Placement};
use bande_keyboard::Layouts;
use bande_status::devices::can_sleep;
use bande_toolkit::SessionEnd;
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;

use crate::L10N;
use crate::about::{About, AboutCommand, follow_about};
use crate::backup::{BackupCommand, BackupUpdate, follow_backup};
use crate::battery::{BatteryCommand, BatteryState, follow_battery};
use crate::brightness::follow_brightness;
use crate::client::{CONFIG_SETTINGS, SettingsClient};
use crate::consent::{KeyboardAccess, follow_keyboard_access};
use crate::crash_reports::{CrashReport, CrashReportCommand, follow_crash_reports};
use crate::defaults::{DefaultApp, DefaultAppCommand, follow_default_apps};
use crate::failure::{explained, pane_failed, sentence};
use crate::locale::{LocaleCommand, LocaleState, follow_locale};
use crate::login::{LoginCommand, LoginItems, follow_login_items};
use crate::lookup::{LookUpCommand, LookUpState, follow_look_up};
use crate::notifications::{AppNotifications, NotificationCommand, follow_app_notifications};
use crate::passwords::{Concealed, PasswordCommand, Saved, follow_passwords};
use crate::permissions::{Grants, PermissionCommand, follow_permissions};
use crate::pictures::{Pair, PictureCommand, follow_pictures};
use crate::pointing::{PointingDevices, UDEV_DATA, follow_pointing};
use crate::printers::{PrinterCommand, PrintersState, follow_printers};
use crate::privacy::clear_clipboard;
use crate::privilege::{Privilege, follow_privileges};
use crate::security_status::{SecurityStatus, follow_security_status};
use crate::services::{
    AudioCommand, BluetoothCommand, NetworkCommand, follow_audio, follow_bluetooth, follow_network,
};
use crate::sharing::{SharingCommand, SharingState, follow_sharing};
use crate::software::{SoftwareCommand, SoftwareState, follow_software};
use crate::spelling::{SpellingCommand, SpellingState, follow_spelling};
use crate::state::{Answer, BluetoothState, Joined, NetworkState, Question, WifiState};
use crate::storage::{Storage, StorageCommand, follow_storage};
use crate::time::{TimeCommand, TimeState, follow_time};
use crate::users::{Account, PasswordOutcome, UsersCommand, follow_users};
use bande_network::import_vpn;
use bande_settings_panes::{PaneId, SettingId, Value};
use bande_status::brightness::{DisplayLevel, LevelRequest};
use bande_watch::FolderWatch;

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";

#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Values(Vec<(SettingId, Value)>),
    Displays(BTreeMap<String, DisplaySettings>),
    DisplayLevels(Vec<DisplayLevel>),
    Arrangements(Vec<Vec<Placement>>),
    Audio(Option<bande_audio::Snapshot>),
    Wifi(Option<WifiState>),
    Network(NetworkState),
    Joined(Vec<u8>, Joined),
    Bluetooth(Option<BluetoothState>),
    Pairing(Option<Question>),
    Pictures(Vec<(String, Pair)>),
    Thumbnail(Pair, bande_wallpaper::Picture),
    About(About),
    Storage(Storage),
    Backup(BackupUpdate),
    CrashReports(Vec<CrashReport>),
    CrashReportsChanged,
    Time(Option<TimeState>),
    Login(LoginItems),
    Battery(Option<BatteryState>),
    CannotSleep,
    NotificationApps(Vec<AppNotifications>),
    Software(SoftwareState),
    SoftwareChecked(Option<i64>),
    Locale(Option<LocaleState>),
    Layouts(Layouts),
    Pointing(PointingDevices),
    InputLevel(f32),
    Failed(SettingId, String),
    PaneFailed(PaneId, String),
    Refused(Vec<Privilege>),
    DefaultApps(Vec<DefaultApp>),
    Permissions(Grants),
    KeyboardAccess(Vec<KeyboardAccess>),
    Spelling(SpellingState),
    LookUp(LookUpState),
    Security(SecurityStatus),
    Printers(PrintersState),
    Sharing(SharingState),
    PrinterAdded(bool),
    PrintingInstalled,
    Accounts(Vec<Account>, bool),
    Password(PasswordOutcome),
    PictureChosen(String, bande_toolkit::Image),
    AccountCreated(Option<String>),
    Passwords(bool, Vec<Saved>),
    RevealedPassword(String, Concealed),
    CopiedPassword(Concealed),
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
    Backup(BackupCommand),
    CrashReports(CrashReportCommand),
    CopyText(String),
    Time(TimeCommand),
    Login(LoginCommand),
    Battery(BatteryCommand),
    Notifications(NotificationCommand),
    Software(SoftwareCommand),
    Locale(LocaleCommand),
    DefaultApp(DefaultAppCommand),
    Permission(PermissionCommand),
    KeyboardAccess(String, bool),
    Spelling(SpellingCommand),
    LookUp(LookUpCommand),
    Printers(PrinterCommand),
    Sharing(SharingCommand),
    Users(UsersCommand),
    Brightness(LevelRequest),
    Reveal(PathBuf),
    EndSession(SessionEnd),
    ImportVpn,
    ExcludeFolder,
    ClearClipboard,
    Passwords(PasswordCommand),
    CopySecret(Concealed),
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

fn hide_sleep_if_unavailable(system: &Connection, updates: Updates) {
    let system = system.clone();
    spawn("can-sleep", move || {
        if !can_sleep(&system) {
            updates.send(Update::CannotSleep);
        }
    });
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
        match (client.displays(), client.arrangements()) {
            (Ok(displays), Ok(arrangements)) => {
                read.push(Update::Displays(displays));
                read.push(Update::Arrangements(arrangements));
            }
            (Err(error), _) | (_, Err(error)) => {
                tracing::warn!("cannot read the display settings: {error}")
            }
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

fn exclude_folder(session: &Connection) -> zbus::Result<()> {
    let Some(folder) = choose_file(session, &L10N.tr("choose-excluded-folder"), None)? else {
        return Ok(());
    };
    let client = SettingsClient::connect(session)?;
    let mut excluded = match client.get(SettingId::SearchExcluded)? {
        Value::Codes(paths) => paths,
        _ => Vec::new(),
    };
    let folder = match std::env::home_dir().and_then(|home| {
        folder
            .strip_prefix(home)
            .ok()
            .filter(|rest| !rest.as_os_str().is_empty())
            .map(|rest| Path::new("~").join(rest))
    }) {
        Some(under_home) => under_home,
        None => folder,
    }
    .to_string_lossy()
    .into_owned();
    if excluded.contains(&folder) {
        return Ok(());
    }
    excluded.push(folder);
    client.set(SettingId::SearchExcluded, &Value::Codes(excluded))
}

fn replaces(queued: &Command, command: &Command) -> bool {
    match (queued, command) {
        (Command::Set(queued, _), Command::Set(id, _)) => queued == id,
        (Command::Display(queued, _), Command::Display(identity, _)) => queued == identity,
        (Command::Arrangement(queued), Command::Arrangement(placements)) => {
            bande_config::same_displays(
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
                    updates.send(match restore.first() {
                        Some(setting) => {
                            Update::Failed(*setting, sentence(&error, "service-desktop"))
                        }
                        None => Update::PaneFailed(
                            PaneId::Displays,
                            explained(&error, "service-desktop", "displays-change-failed"),
                        ),
                    });
                    updates.send(Update::Values(client.read(&restore)));
                    if let Ok(displays) = client.displays() {
                        updates.send(Update::Displays(displays));
                    }
                    if let Ok(arrangements) = client.arrangements() {
                        updates.send(Update::Arrangements(arrangements));
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
    backup: Option<Sender<BackupCommand>>,
    crash_reports: Sender<CrashReportCommand>,
    time: Option<Sender<TimeCommand>>,
    login: Sender<LoginCommand>,
    battery: Option<Sender<BatteryCommand>>,
    notifications: Sender<NotificationCommand>,
    software: Option<Sender<SoftwareCommand>>,
    locale: Option<Sender<LocaleCommand>>,
    default_apps: Sender<DefaultAppCommand>,
    permissions: Sender<PermissionCommand>,
    keyboard_access: Sender<(String, bool)>,
    spelling: Sender<SpellingCommand>,
    look_up: Sender<LookUpCommand>,
    passwords: Sender<PasswordCommand>,
    brightness: Option<Sender<LevelRequest>>,
    printers: Option<Sender<PrinterCommand>>,
    sharing: Sender<SharingCommand>,
    users: Sender<UsersCommand>,
    _pointing: Option<FolderWatch>,
    _security: Option<FolderWatch>,
}

impl Services {
    pub fn start(session: &Connection, system: Option<&Connection>, updates: &Updates) -> Self {
        follow_config(session, updates.clone());
        follow_notifications(session, updates.clone());
        let layouts = updates.clone();
        spawn("layouts", move || {
            layouts.send(Update::Layouts(Layouts::load()));
        });
        if let Some(system) = system {
            follow_privileges(system, updates.clone());
            hide_sleep_if_unavailable(system, updates.clone());
        }
        Self {
            connection: session.clone(),
            updates: updates.clone(),
            session: worker(session, updates.clone()),
            audio: follow_audio(updates.clone()),
            network: system.map(|system| follow_network(system, updates.clone())),
            bluetooth: system.map(|system| follow_bluetooth(system, updates.clone())),
            pictures: follow_pictures(session, bande_xdg::data_dirs(), updates.clone()),
            about: system.map(|system| follow_about(system, updates.clone())),
            storage: follow_storage(updates.clone()),
            backup: system.map(|system| follow_backup(system, updates.clone())),
            crash_reports: follow_crash_reports(updates.clone()),
            time: system.map(|system| follow_time(system, updates.clone())),
            login: follow_login_items(updates.clone()),
            battery: system.map(|system| follow_battery(system, updates.clone())),
            notifications: follow_app_notifications(session, updates.clone()),
            software: system.map(|system| follow_software(session, system, updates.clone())),
            locale: system.map(|system| follow_locale(system, updates.clone())),
            default_apps: follow_default_apps(updates.clone()),
            permissions: follow_permissions(session, updates.clone()),
            keyboard_access: follow_keyboard_access(updates.clone()),
            spelling: follow_spelling(updates.clone()),
            look_up: follow_look_up(updates.clone()),
            _security: follow_security_status(system, updates.clone()),
            passwords: follow_passwords(session, system, updates.clone()),
            brightness: follow_brightness(session, updates.clone()),
            printers: system.map(|system| follow_printers(system, session, updates.clone())),
            sharing: follow_sharing(Some(session.clone()), system.cloned(), updates.clone()),
            users: follow_users(session, system, updates.clone()),
            _pointing: follow_pointing(PathBuf::from(UDEV_DATA), updates.clone())
                .inspect_err(|error| tracing::warn!("cannot watch {UDEV_DATA}: {error}"))
                .ok(),
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
            Command::Backup(command) => self
                .backup
                .as_ref()
                .is_some_and(|backup| backup.send(command).is_ok()),
            Command::CrashReports(command) => self.crash_reports.send(command).is_ok(),
            Command::Time(command) => self
                .time
                .as_ref()
                .is_some_and(|time| time.send(command).is_ok()),
            Command::Login(command) => self.login.send(command).is_ok(),
            Command::DefaultApp(command) => self.default_apps.send(command).is_ok(),
            Command::Permission(command) => self.permissions.send(command).is_ok(),
            Command::KeyboardAccess(line, on) => self.keyboard_access.send((line, on)).is_ok(),
            Command::Spelling(command) => self.spelling.send(command).is_ok(),
            Command::LookUp(command) => self.look_up.send(command).is_ok(),
            Command::Passwords(command) => self.passwords.send(command).is_ok(),
            Command::Users(command) => self.users.send(command).is_ok(),
            Command::Sharing(command) => self.sharing.send(command).is_ok(),
            Command::Printers(command) => self
                .printers
                .as_ref()
                .is_some_and(|printers| printers.send(command).is_ok()),
            Command::Brightness(request) => self
                .brightness
                .as_ref()
                .is_some_and(|brightness| brightness.send(request).is_ok()),
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
            Command::Reveal(_)
            | Command::EndSession(_)
            | Command::CopySecret(_)
            | Command::CopyText(_) => false,
            Command::ClearClipboard => {
                let (session, updates) = (self.connection.clone(), self.updates.clone());
                spawn("clear-clipboard", move || {
                    if let Err(error) = clear_clipboard(&session) {
                        updates.send(pane_failed(
                            PaneId::PrivacySecurity,
                            "clipboard-clear-failed",
                            error,
                        ));
                    }
                });
                true
            }
            Command::ExcludeFolder => {
                let (session, updates) = (self.connection.clone(), self.updates.clone());
                spawn("exclude-folder", move || {
                    if let Err(error) = exclude_folder(&session) {
                        updates.send(pane_failed(PaneId::Search, "search-exclude-failed", error));
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
                        Some((
                            L10N.tr("vpn-files"),
                            vec![(0, "*.conf".to_owned()), (0, "*.ovpn".to_owned())],
                        )),
                    );
                    let imported = match chosen {
                        Ok(Some(file)) => import_vpn(&file).map_err(|error| error.to_string()),
                        Ok(None) => Ok(()),
                        Err(error) => Err(error.to_string()),
                    };
                    if let Err(error) = imported {
                        updates.send(pane_failed(PaneId::Network, "vpn-import-failed", error));
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
