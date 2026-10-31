mod alarm;
mod center;
pub mod client;
mod wire;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

use crate::alarm::Alarm;

pub use center::{
    App, AppRule, BANNER_DURATION, BANNER_LINGER, Banners, Center, CloseReason, HiddenBanner,
    Invoked, Notification, Presentation, Stack, Timeout, Urgency,
};
pub use wire::{WireNotification, WirePresentation, WireStack};

pub const BUS_NAME: &str = "org.freedesktop.Notifications";
pub const OBJECT_PATH: &str = "/org/freedesktop/Notifications";
pub const SPEC_VERSION: &str = "1.2";
const APPS_FILE: &str = "notification-apps";

struct State {
    center: Center,
    presented: Presentation,
    presentations: Sender<Presentation>,
    stopped: bool,
}

impl State {
    fn present(&mut self) {
        let presentation = self.center.presentation();
        if presentation != self.presented {
            let _ = self.presentations.send(presentation.clone());
            self.presented = presentation;
        }
    }
}

struct Shared {
    state: Mutex<State>,
    alarm: Alarm,
    do_not_disturb_file: Option<PathBuf>,
    apps_file: Option<PathBuf>,
}

pub fn do_not_disturb_path(xdg_state_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    let base = match xdg_state_home.filter(|path| path.is_absolute()) {
        Some(state_home) => state_home.to_path_buf(),
        None => home?.join(".local").join("state"),
    };
    Some(base.join("unbound").join("do-not-disturb"))
}

pub type WireApp = (String, String, bool, String, bool);

fn wire_app(app: &App) -> WireApp {
    (
        app.group.clone(),
        app.name.clone(),
        app.rule.allow,
        app.rule.banners.keyword().to_owned(),
        app.rule.in_center,
    )
}

pub fn write_apps<'a>(apps: impl IntoIterator<Item = &'a App>) -> String {
    let storable = |text: &str| !text.contains(['\t', '\n', '\r']);
    apps.into_iter()
        .filter(|app| storable(&app.group) && storable(&app.name))
        .map(|app| {
            let (group, name, allow, banners, center) = wire_app(app);
            format!("{group}\t{name}\t{allow}\t{banners}\t{center}\n")
        })
        .collect()
}

pub fn read_apps(text: &str) -> Vec<App> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let group = fields.next().filter(|group| !group.is_empty())?.to_owned();
            let name = fields.next()?.to_owned();
            let allow = fields.next()?.parse().ok()?;
            let banners = Banners::parse(fields.next()?)?;
            let in_center = fields.next()?.parse().ok()?;
            Some(App {
                group,
                name,
                rule: AppRule {
                    allow,
                    banners,
                    in_center,
                },
            })
        })
        .collect()
}

fn remember_do_not_disturb(file: &Path, enabled: bool) -> io::Result<()> {
    if !enabled {
        return match fs::remove_file(file) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            removed => removed,
        };
    }
    if let Some(directory) = file.parent() {
        fs::create_dir_all(directory)?;
    }
    fs::write(file, b"")
}

enum Outcome {
    Invoked { closed: bool },
    Closed(Vec<u32>),
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update<T>(&self, change: impl FnOnce(&mut Center) -> T) -> T {
        let mut state = self.lock();
        let result = change(&mut state.center);
        state.present();
        self.alarm.nudge();
        result
    }

    fn remember_apps(&self) {
        let Some(file) = &self.apps_file else {
            return;
        };
        let apps = write_apps(self.lock().center.apps());
        let written = file
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| fs::write(file, apps));
        if let Err(error) = written {
            tracing::warn!(
                "cannot remember notification apps in {}: {error}",
                file.display()
            );
        }
    }

    fn set_do_not_disturb(&self, enabled: bool) {
        self.update(|center| center.set_do_not_disturb(enabled));
        if let Some(file) = &self.do_not_disturb_file
            && let Err(error) = remember_do_not_disturb(file, enabled)
        {
            tracing::warn!(
                "cannot remember Do Not Disturb in {}: {error}",
                file.display()
            );
        }
    }

    fn activate(&self, id: u32, action_key: &str) -> Outcome {
        self.update(|center| {
            if let Some(Invoked { closed }) = center.invoke(id, action_key) {
                return Outcome::Invoked { closed };
            }
            let resident = center.get(id).is_some_and(|found| found.resident);
            if resident {
                center.hide_banner(id);
                Outcome::Closed(Vec::new())
            } else {
                Outcome::Closed(Vec::from_iter(center.close(id).then_some(id)))
            }
        })
    }
}

async fn emit_closed(emitter: &SignalEmitter<'_>, ids: &[u32], reason: CloseReason) {
    for id in ids {
        if let Err(error) = Notifications::notification_closed(emitter, *id, reason as u32).await {
            tracing::warn!("cannot announce that notification {id} closed: {error}");
        }
    }
}

async fn announce(
    emitter: &SignalEmitter<'_>,
    id: u32,
    action_key: &str,
    activation_token: &str,
    outcome: &Outcome,
) -> bool {
    match outcome {
        Outcome::Invoked { closed } => {
            if !activation_token.is_empty() {
                let _ = Notifications::activation_token(emitter, id, activation_token).await;
            }
            let _ = Notifications::action_invoked(emitter, id, action_key).await;
            if *closed {
                emit_closed(emitter, &[id], CloseReason::Dismissed).await;
            }
            true
        }
        Outcome::Closed(ids) => {
            emit_closed(emitter, ids, CloseReason::Dismissed).await;
            false
        }
    }
}

pub struct NotificationServer {
    connection: Connection,
    shared: Arc<Shared>,
}

impl NotificationServer {
    pub fn start(connection: &Connection) -> zbus::Result<Self> {
        Self::launch(connection, None)
    }

    pub fn start_remembering(
        connection: &Connection,
        do_not_disturb_file: PathBuf,
    ) -> zbus::Result<Self> {
        Self::launch(connection, Some(do_not_disturb_file))
    }

    fn launch(connection: &Connection, do_not_disturb_file: Option<PathBuf>) -> zbus::Result<Self> {
        let mut center = Center::default();
        center.set_do_not_disturb(do_not_disturb_file.as_deref().is_some_and(Path::exists));
        let apps_file = do_not_disturb_file
            .as_deref()
            .map(|file| file.with_file_name(APPS_FILE));
        if let Some(file) = &apps_file {
            for app in read_apps(&fs::read_to_string(file).unwrap_or_default()) {
                center.remember(app);
            }
        }
        let (presentations, presented) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                presented: center.presentation(),
                center,
                presentations,
                stopped: false,
            }),
            alarm: Alarm::new().map_err(|error| zbus::Error::InputOutput(error.into()))?,
            do_not_disturb_file,
            apps_file,
        });
        connection.object_server().at(
            OBJECT_PATH,
            Notifications {
                shared: Arc::clone(&shared),
                sounds: Mutex::new(sounds(connection)),
            },
        )?;
        connection.object_server().at(
            OBJECT_PATH,
            NotificationCenter {
                shared: Arc::clone(&shared),
            },
        )?;
        connection.request_name(BUS_NAME)?;
        let timer = Arc::clone(&shared);
        let timer_connection = connection.clone();
        thread::spawn(move || expire(&timer, &timer_connection));
        let publisher = connection.clone();
        thread::spawn(move || publish(&presented, &publisher));
        Ok(Self {
            connection: connection.clone(),
            shared,
        })
    }

    pub fn apps(&self) -> Vec<App> {
        self.shared.lock().center.apps().cloned().collect()
    }

    pub fn active(&self) -> Vec<Notification> {
        self.shared.lock().center.notifications().cloned().collect()
    }

    pub fn dismiss(&self, id: u32) -> zbus::Result<bool> {
        let closed = self.shared.update(|center| center.close(id));
        if closed {
            let emitter = SignalEmitter::new(self.connection.inner(), OBJECT_PATH)?;
            zbus::block_on(emit_closed(&emitter, &[id], CloseReason::Dismissed));
        }
        Ok(closed)
    }
}

impl Drop for NotificationServer {
    fn drop(&mut self) {
        self.shared.lock().stopped = true;
        self.shared.alarm.nudge();
    }
}

fn publish(presented: &Receiver<Presentation>, connection: &Connection) {
    let Ok(emitter) = SignalEmitter::new(connection.inner(), OBJECT_PATH) else {
        return;
    };
    while let Ok(mut presentation) = presented.recv() {
        while let Ok(newer) = presented.try_recv() {
            presentation = newer;
        }
        let wire = presentation.to_wire();
        if let Err(error) = zbus::block_on(NotificationCenter::presentation_changed(&emitter, wire))
        {
            tracing::warn!("cannot publish the notifications on screen: {error}");
        }
    }
}

fn expire(shared: &Shared, connection: &Connection) {
    loop {
        let mut state = shared.lock();
        if state.stopped {
            return;
        }
        let expired = state.center.advance(alarm::now());
        state.present();
        let deadline = state.center.next_deadline();
        drop(state);
        if !expired.is_empty() {
            if let Ok(emitter) = SignalEmitter::new(connection.inner(), OBJECT_PATH) {
                zbus::block_on(emit_closed(&emitter, &expired, CloseReason::Expired));
            }
            continue;
        }
        if let Err(error) = shared.alarm.wait_until(deadline) {
            tracing::error!("cannot wait for notifications to expire: {error}");
            return;
        }
    }
}

fn hint<'a, T: TryFrom<&'a OwnedValue>>(
    hints: &'a HashMap<String, OwnedValue>,
    name: &str,
) -> Option<T> {
    hints.get(name).and_then(|value| T::try_from(value).ok())
}

const MESSAGE_SOUND: &str = "message-new-instant";

fn sounds(connection: &Connection) -> Sender<String> {
    let (sender, names) = mpsc::channel::<String>();
    let connection = connection.clone();
    thread::spawn(move || {
        for name in names {
            unbound_sound::play(&connection, &name);
        }
    });
    sender
}

struct Notifications {
    shared: Arc<Shared>,
    sounds: Mutex<Sender<String>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    fn get_capabilities(&self) -> Vec<String> {
        ["actions", "body", "persistence", "sound"]
            .map(str::to_owned)
            .to_vec()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        let sound = hint::<&str>(&hints, "sound-name")
            .filter(|name| !name.is_empty())
            .unwrap_or(MESSAGE_SOUND)
            .to_owned();
        let silent = hint(&hints, "suppress-sound").unwrap_or(false);
        let notification = Notification {
            id: replaces_id,
            app_name,
            app_icon,
            summary,
            body,
            actions: actions
                .as_chunks::<2>()
                .0
                .iter()
                .map(|[key, label]| (key.clone(), label.clone()))
                .collect(),
            urgency: wire::urgency_of(hint::<u8>(&hints, "urgency")),
            desktop_entry: hint::<&str>(&hints, "desktop-entry")
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned),
            resident: hint(&hints, "resident").unwrap_or(false),
            transient: hint(&hints, "transient").unwrap_or(false),
        };
        let (id, audible, new_app) = self.shared.update(|center| {
            let known = center.apps().count();
            let fresh = replaces_id == 0 || center.get(replaces_id).is_none();
            let shown = !center.do_not_disturb() || notification.urgency == Urgency::Critical;
            let id = center.post(
                notification,
                Timeout::from_milliseconds(expire_timeout),
                alarm::now(),
            );
            let delivered = center.get(id).is_some();
            (
                id,
                fresh && shown && !silent && delivered,
                center.apps().count() != known,
            )
        });
        if new_app {
            self.shared.remember_apps();
        }
        if audible {
            let _ = self
                .sounds
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(sound);
        }
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if !self.shared.update(|center| center.close(id)) {
            return Err(zbus::fdo::Error::Failed(String::new()));
        }
        Self::notification_closed(&emitter, id, CloseReason::Closed as u32).await?;
        Ok(())
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Unbound".to_owned(),
            "Unbound".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
            SPEC_VERSION.to_owned(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn activation_token(
        emitter: &SignalEmitter<'_>,
        id: u32,
        activation_token: &str,
    ) -> zbus::Result<()>;
}

struct NotificationCenter {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.unbound.NotificationCenter")]
impl NotificationCenter {
    #[zbus(property)]
    fn do_not_disturb(&self) -> bool {
        self.shared.lock().center.do_not_disturb()
    }

    #[zbus(property)]
    fn set_do_not_disturb(&mut self, enabled: bool) {
        self.shared.set_do_not_disturb(enabled);
    }

    fn get_apps(&self) -> Vec<WireApp> {
        self.shared.lock().center.apps().map(wire_app).collect()
    }

    fn set_app_rule(
        &self,
        group: String,
        allow: bool,
        banners: String,
        in_center: bool,
    ) -> zbus::fdo::Result<()> {
        let banners = Banners::parse(&banners).ok_or_else(|| {
            zbus::fdo::Error::InvalidArgs(format!("unknown banner style {banners:?}"))
        })?;
        let rule = AppRule {
            allow,
            banners,
            in_center,
        };
        let known = self.shared.update(|center| center.set_rule(&group, rule));
        if !known {
            return Err(zbus::fdo::Error::InvalidArgs(format!("no app {group:?}")));
        }
        self.shared.remember_apps();
        Ok(())
    }

    fn get_presentation(&self) -> WirePresentation {
        self.shared.lock().center.presentation().to_wire()
    }

    async fn activate(
        &self,
        id: u32,
        action_key: String,
        activation_token: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> bool {
        let outcome = self.shared.activate(id, &action_key);
        announce(&emitter, id, &action_key, &activation_token, &outcome).await
    }

    async fn dismiss(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        if self.shared.update(|center| center.close(id)) {
            emit_closed(&emitter, &[id], CloseReason::Dismissed).await;
        }
    }

    async fn hide_banner(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        if let Some(HiddenBanner { closed: true }) =
            self.shared.update(|center| center.hide_banner(id))
        {
            emit_closed(&emitter, &[id], CloseReason::Dismissed).await;
        }
    }

    async fn clear(&self, group: String, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let cleared = self.shared.update(|center| center.clear(&group));
        emit_closed(&emitter, &cleared, CloseReason::Dismissed).await;
    }

    async fn clear_all(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let cleared = self.shared.update(Center::clear_all);
        emit_closed(&emitter, &cleared, CloseReason::Dismissed).await;
    }

    fn hold_banners(&self, held: bool) {
        self.shared
            .update(|center| center.hold_banners(held, alarm::now()));
    }

    #[zbus(signal)]
    async fn presentation_changed(
        emitter: &SignalEmitter<'_>,
        presentation: WirePresentation,
    ) -> zbus::Result<()>;
}
