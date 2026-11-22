use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_keyboard::{
    GlobalShortcut, Trigger, assign, read_global_shortcuts, write_global_shortcuts,
};
use bande_watch::{Changes, FolderWatch};
use zbus::MatchRule;
use zbus::message::Type as MessageType;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::file_chooser::option;
use crate::request::{self, CANCELLED, Cancellation, OTHER, SUCCESS};
use crate::{OBJECT_PATH, from_frontend};

pub const COMPOSITOR_NAME: &str = "org.bande.GlobalShortcuts";
pub const COMPOSITOR_PATH: &str = "/org/bande/GlobalShortcuts";
const INTERFACE: &str = "org.freedesktop.impl.portal.GlobalShortcuts";
const RELAYED: [&str; 2] = ["Activated", "Deactivated"];

type Options = HashMap<String, OwnedValue>;
type Described = (String, HashMap<String, OwnedValue>);
type Sessions = Arc<Mutex<HashMap<OwnedObjectPath, String>>>;
type Requested = (String, String, Option<Trigger>);

const MAX_SHORTCUTS: usize = 32;
const MAX_ID_BYTES: usize = 64;
const MAX_DESCRIPTION_CHARS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: String,
    pub description: String,
    pub trigger: Option<Trigger>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Consent {
    pub app_id: String,
    pub configuring: bool,
    pub shortcuts: Vec<Proposal>,
    pub taken: Vec<Trigger>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Consented {
    Chosen(Vec<(String, Option<Trigger>)>),
    Declined,
    Unavailable,
}

pub trait ShortcutsUi: Send + Sync {
    fn ask(&self, consent: &Consent, cancellation: &Cancellation) -> Consented;
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read_file(path: &Path) -> io::Result<Vec<GlobalShortcut>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(read_global_shortcuts(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn is_of(shortcut: &GlobalShortcut, app_id: &str, id: &str) -> bool {
    shortcut.app_id == app_id && shortcut.id == id
}

/// The shortcuts the user has not decided on yet, each with its preferred trigger when an app
/// may have it and nobody has it.
pub fn proposals(known: &[GlobalShortcut], app_id: &str, requested: &[Requested]) -> Vec<Proposal> {
    let mut proposed: Vec<Proposal> = Vec::new();
    for (id, description, preferred) in requested {
        if known.iter().any(|shortcut| is_of(shortcut, app_id, id))
            || proposed.iter().any(|proposal| proposal.id == *id)
        {
            continue;
        }
        let trigger = preferred.filter(|trigger| {
            trigger.assignable()
                && !known
                    .iter()
                    .any(|shortcut| shortcut.trigger == Some(*trigger))
                && !proposed
                    .iter()
                    .any(|proposal| proposal.trigger == Some(*trigger))
        });
        proposed.push(Proposal {
            id: id.clone(),
            description: description.clone(),
            trigger,
        });
    }
    proposed
}

/// Keeps what the user decided for an app's shortcuts, giving a shortcut its trigger only when an
/// app may have it and no other shortcut has it.
pub fn decide(
    mut known: Vec<GlobalShortcut>,
    app_id: &str,
    proposed: &[Proposal],
    decisions: Vec<(String, Option<Trigger>)>,
) -> Vec<GlobalShortcut> {
    for (id, trigger) in decisions {
        if !known.iter().any(|shortcut| is_of(shortcut, app_id, &id)) {
            let Some(proposal) = proposed.iter().find(|proposal| proposal.id == id) else {
                continue;
            };
            known.push(GlobalShortcut {
                app_id: app_id.to_owned(),
                id: id.clone(),
                description: proposal.description.clone(),
                trigger: None,
            });
        }
        if !assign(&mut known, app_id, &id, trigger) {
            assign(&mut known, app_id, &id, None);
        }
    }
    known
}

fn described(shortcut: &GlobalShortcut) -> Described {
    let mut details = HashMap::from([(
        "description".to_owned(),
        OwnedValue::from(zbus::zvariant::Str::from(shortcut.description.clone())),
    )]);
    if let Some(trigger) = shortcut.trigger {
        details.insert(
            "trigger_description".to_owned(),
            OwnedValue::from(zbus::zvariant::Str::from(trigger.keys())),
        );
    }
    (shortcut.id.clone(), details)
}

fn of_app(all: &[GlobalShortcut], app_id: &str) -> Vec<Described> {
    all.iter()
        .filter(|shortcut| shortcut.app_id == app_id)
        .map(described)
        .collect()
}

fn results(shortcuts: Vec<Described>) -> HashMap<String, OwnedValue> {
    let value = Value::from(shortcuts);
    match OwnedValue::try_from(value) {
        Ok(value) => HashMap::from([("shortcuts".to_owned(), value)]),
        Err(error) => {
            tracing::warn!("cannot describe the shortcuts: {error}");
            HashMap::new()
        }
    }
}

#[derive(Clone)]
struct Store {
    file: Option<PathBuf>,
    announced: Arc<Mutex<Vec<GlobalShortcut>>>,
}

impl Store {
    fn read(&self) -> io::Result<Vec<GlobalShortcut>> {
        self.change(None, |known| known)
    }

    fn change(
        &self,
        quiet: Option<&str>,
        change: impl FnOnce(Vec<GlobalShortcut>) -> Vec<GlobalShortcut>,
    ) -> io::Result<Vec<GlobalShortcut>> {
        let Some(file) = &self.file else {
            return Err(io::Error::other("there is no state folder"));
        };
        let mut after = Vec::new();
        bande_xdg::change_file(file, |text| {
            let before = read_global_shortcuts(text);
            after = change(before.clone());
            if let Some(app_id) = quiet {
                let mut announced = lock(&self.announced);
                announced.retain(|shortcut| shortcut.app_id != app_id);
                announced.extend(
                    after
                        .iter()
                        .filter(|shortcut| shortcut.app_id == app_id)
                        .cloned(),
                );
            }
            Ok(match after == before {
                true => text.to_owned(),
                false => write_global_shortcuts(&after),
            })
        })?;
        Ok(after)
    }
}

#[derive(Clone)]
pub struct GlobalShortcuts {
    sessions: Sessions,
    store: Store,
    ui: Arc<dyn ShortcutsUi>,
    _watch: Option<Arc<FolderWatch>>,
}

struct Session {
    sessions: Sessions,
    path: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Session")]
impl Session {
    async fn close(
        &self,
        #[zbus(object_server)] server: &ObjectServer,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        lock(&self.sessions).remove(&self.path);
        bande_bus::warn_unsent("Closed", Self::closed(&emitter).await);
        if let Err(error) = server.remove::<Self, _>(&self.path).await {
            tracing::warn!("cannot close the shortcuts session {}: {error}", self.path);
        }
    }

    #[zbus(signal)]
    async fn closed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

fn relay(session: &zbus::blocking::Connection, sessions: &Sessions) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .sender(COMPOSITOR_NAME)?
        .interface(COMPOSITOR_NAME)?
        .path(COMPOSITOR_PATH)?
        .build();
    let signals = zbus::blocking::MessageIterator::for_match_rule(rule, session, None)?;
    for message in signals.flatten() {
        let header = message.header();
        let Some(member) = header
            .member()
            .map(|member| member.to_string())
            .filter(|member| RELAYED.contains(&member.as_str()))
        else {
            continue;
        };
        let Ok((app_id, id, timestamp)) = message.body().deserialize::<(String, String, u64)>()
        else {
            continue;
        };
        for path in sessions_of(sessions, &app_id) {
            bande_bus::warn_unsent(
                &member,
                session.emit_signal(
                    None::<&str>,
                    OBJECT_PATH,
                    INTERFACE,
                    member.as_str(),
                    &(
                        path,
                        id.as_str(),
                        timestamp,
                        HashMap::<&str, Value<'_>>::new(),
                    ),
                ),
            );
        }
    }
    Ok(())
}

fn sessions_of(sessions: &Sessions, app_id: &str) -> Vec<OwnedObjectPath> {
    lock(sessions)
        .iter()
        .filter(|(_, owner)| **owner == app_id)
        .map(|(path, _)| path.clone())
        .collect()
}

fn announce_changes(session: &zbus::blocking::Connection, sessions: &Sessions, store: &Store) {
    let now = match store.file.as_deref().map(read_file) {
        Some(Ok(now)) => now,
        Some(Err(error)) => {
            tracing::warn!("cannot read the apps' shortcuts: {error}");
            return;
        }
        None => return,
    };
    let before = std::mem::replace(&mut *lock(&store.announced), now.clone());
    let apps: Vec<String> = lock(sessions).values().cloned().collect();
    for app_id in apps {
        let shortcuts = of_app(&now, &app_id);
        if shortcuts == of_app(&before, &app_id) {
            continue;
        }
        for path in sessions_of(sessions, &app_id) {
            bande_bus::warn_unsent(
                "ShortcutsChanged",
                session.emit_signal(
                    None::<&str>,
                    OBJECT_PATH,
                    INTERFACE,
                    "ShortcutsChanged",
                    &(path, shortcuts.clone()),
                ),
            );
        }
    }
}

fn follow_changes(
    session: &zbus::blocking::Connection,
    sessions: &Sessions,
    store: &Store,
) -> io::Result<Option<FolderWatch>> {
    let Some(file) = store.file.clone() else {
        return Ok(None);
    };
    let Some(folder) = file.parent().map(Path::to_path_buf) else {
        return Ok(None);
    };
    std::fs::create_dir_all(&folder)?;
    *lock(&store.announced) = read_file(&file)?;
    let (session, sessions, store) = (session.clone(), Arc::clone(sessions), store.clone());
    let watch = FolderWatch::spawn(Changes::Saved, move |changed| {
        if changed.contains(&file) {
            announce_changes(&session, &sessions, &store);
        }
    })?;
    watch.watch(&folder)?;
    Ok(Some(watch))
}

fn requested_of(shortcuts: &[Described]) -> Vec<Requested> {
    shortcuts
        .iter()
        .filter(|(id, _)| !id.is_empty() && id.len() <= MAX_ID_BYTES)
        .take(MAX_SHORTCUTS)
        .map(|(id, details)| {
            let description: String = option(details, "description").unwrap_or_default();
            let preferred: Option<String> = option(details, "preferred_trigger");
            (
                id.clone(),
                description.chars().take(MAX_DESCRIPTION_CHARS).collect(),
                preferred.as_deref().and_then(Trigger::parse),
            )
        })
        .collect()
}

fn taken_by_others(known: &[GlobalShortcut], app_id: &str) -> Vec<Trigger> {
    known
        .iter()
        .filter(|shortcut| shortcut.app_id != app_id)
        .filter_map(|shortcut| shortcut.trigger)
        .collect()
}

fn configure(store: &Store, ui: &dyn ShortcutsUi, app_id: String) -> io::Result<()> {
    let known = store.read()?;
    let shortcuts: Vec<Proposal> = known
        .iter()
        .filter(|shortcut| shortcut.app_id == app_id)
        .map(|shortcut| Proposal {
            id: shortcut.id.clone(),
            description: shortcut.description.clone(),
            trigger: shortcut.trigger,
        })
        .collect();
    if shortcuts.is_empty() {
        tracing::info!("{app_id} has no shortcuts to configure");
        return Ok(());
    }
    let consent = Consent {
        taken: taken_by_others(&known, &app_id),
        app_id,
        configuring: true,
        shortcuts,
    };
    match ui.ask(&consent, &Cancellation::default()) {
        Consented::Chosen(decisions) => {
            store.change(None, |known| {
                decide(known, &consent.app_id, &consent.shortcuts, decisions)
            })?;
        }
        Consented::Declined => {}
        Consented::Unavailable => tracing::warn!("cannot show the shortcuts dialog"),
    }
    Ok(())
}

impl GlobalShortcuts {
    pub fn serve(
        session: &zbus::blocking::Connection,
        ui: Arc<dyn ShortcutsUi>,
    ) -> zbus::Result<()> {
        let store = Store {
            file: bande_xdg::global_shortcuts_file(),
            announced: Arc::default(),
        };
        let sessions = Sessions::default();
        let watch = follow_changes(session, &sessions, &store)
            .inspect_err(|error| {
                tracing::warn!("apps will not hear when their shortcuts change: {error}");
            })
            .ok()
            .flatten();
        let shortcuts = Self {
            sessions,
            store,
            ui,
            _watch: watch.map(Arc::new),
        };
        let (relaying, sessions) = (session.clone(), Arc::clone(&shortcuts.sessions));
        thread::Builder::new()
            .name("global-shortcuts".to_owned())
            .spawn(move || {
                if let Err(error) = relay(&relaying, &sessions) {
                    tracing::warn!("global shortcuts will not reach apps: {error}");
                }
            })
            .map_err(|error| zbus::Error::Failure(error.to_string()))?;
        session.object_server().at(OBJECT_PATH, shortcuts)?;
        Ok(())
    }

    fn app_of(&self, session: &ObjectPath<'_>) -> Option<String> {
        lock(&self.sessions)
            .get(&OwnedObjectPath::from(session.to_owned()))
            .cloned()
    }

    fn bind(
        &self,
        app_id: &str,
        requested: &[Requested],
        cancellation: &Cancellation,
    ) -> io::Result<Option<Vec<GlobalShortcut>>> {
        let known = self.store.read()?;
        let proposed = proposals(&known, app_id, requested);
        let decisions = match proposed.is_empty() {
            true => Vec::new(),
            false => {
                let consent = Consent {
                    app_id: app_id.to_owned(),
                    configuring: false,
                    shortcuts: proposed.clone(),
                    taken: known
                        .iter()
                        .filter_map(|shortcut| shortcut.trigger)
                        .collect(),
                };
                match self.ui.ask(&consent, cancellation) {
                    Consented::Chosen(decisions) => decisions,
                    Consented::Declined => proposed
                        .iter()
                        .map(|proposal| (proposal.id.clone(), None))
                        .collect(),
                    Consented::Unavailable => {
                        return Err(io::Error::other("the shortcuts dialog is unavailable"));
                    }
                }
            }
        };
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        self.store
            .change(Some(app_id), |mut known| {
                for (id, description, _) in requested {
                    if let Some(shortcut) = known
                        .iter_mut()
                        .find(|shortcut| is_of(shortcut, app_id, id))
                    {
                        description.clone_into(&mut shortcut.description);
                    }
                }
                decide(known, app_id, &proposed, decisions)
            })
            .map(Some)
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.GlobalShortcuts")]
impl GlobalShortcuts {
    #[allow(clippy::too_many_arguments)]
    async fn create_session(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        app_id: String,
        _options: Options,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        if !from_frontend(connection, &header).await {
            tracing::warn!("refused global shortcuts that did not come from xdg-desktop-portal");
            return (OTHER, HashMap::new());
        }
        if app_id.is_empty() {
            tracing::info!("refused global shortcuts for an app with no app id");
            return (OTHER, HashMap::new());
        }
        let path = OwnedObjectPath::from(session_handle.to_owned());
        let created = Session {
            sessions: Arc::clone(&self.sessions),
            path: path.clone(),
        };
        if server.at(&path, created).await.is_err() {
            return (OTHER, HashMap::new());
        }
        lock(&self.sessions).insert(path, app_id);
        (SUCCESS, HashMap::new())
    }

    #[allow(clippy::too_many_arguments)]
    async fn bind_shortcuts(
        &self,
        handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        shortcuts: Vec<Described>,
        _parent_window: String,
        _options: Options,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
        #[zbus(object_server)] server: &ObjectServer,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let caller = from_frontend(connection, &header).await;
        let Some(app_id) = self.app_of(&session_handle).filter(|_| caller) else {
            return (OTHER, HashMap::new());
        };
        let requested = requested_of(&shortcuts);
        let ids: Vec<String> = requested.iter().map(|(id, ..)| id.clone()).collect();
        let (this, owner) = (self.clone(), app_id.clone());
        let bound = request::run(server, &handle, move |cancellation| {
            this.bind(&owner, &requested, cancellation)
        })
        .await;
        match bound {
            Ok(Some(Ok(Some(all)))) => (
                SUCCESS,
                results(
                    all.iter()
                        .filter(|shortcut| shortcut.app_id == app_id && ids.contains(&shortcut.id))
                        .map(described)
                        .collect(),
                ),
            ),
            Ok(None | Some(Ok(None))) => (CANCELLED, HashMap::new()),
            Ok(Some(Err(error))) => {
                tracing::warn!("cannot keep the shortcuts of {app_id}: {error}");
                (OTHER, HashMap::new())
            }
            Err(error) => {
                tracing::warn!("cannot ask about the shortcuts of {app_id}: {error}");
                (OTHER, HashMap::new())
            }
        }
    }

    async fn list_shortcuts(
        &self,
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let caller = from_frontend(connection, &header).await;
        let Some(app_id) = self.app_of(&session_handle).filter(|_| caller) else {
            return (OTHER, HashMap::new());
        };
        let store = self.store.clone();
        match blocking::unblock(move || store.read()).await {
            Ok(all) => (SUCCESS, results(of_app(&all, &app_id))),
            Err(error) => {
                tracing::warn!("cannot read the shortcuts of {app_id}: {error}");
                (OTHER, HashMap::new())
            }
        }
    }

    async fn configure_shortcuts(
        &self,
        session_handle: ObjectPath<'_>,
        _parent_window: String,
        _options: Options,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) {
        let caller = from_frontend(connection, &header).await;
        let Some(app_id) = self.app_of(&session_handle).filter(|_| caller) else {
            return;
        };
        let (store, ui) = (self.store.clone(), Arc::clone(&self.ui));
        let started = thread::Builder::new()
            .name("configure-shortcuts".to_owned())
            .spawn(move || {
                if let Err(error) = configure(&store, ui.as_ref(), app_id) {
                    tracing::warn!("cannot change the shortcuts: {error}");
                }
            });
        if let Err(error) = started {
            tracing::warn!("cannot show the shortcuts dialog: {error}");
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(id: &str, trigger: &str) -> Requested {
        (id.to_owned(), String::new(), Trigger::parse(trigger))
    }

    fn accepted(proposed: &[Proposal]) -> Vec<(String, Option<Trigger>)> {
        proposed
            .iter()
            .map(|proposal| (proposal.id.clone(), proposal.trigger))
            .collect()
    }

    #[test]
    fn an_app_gets_only_free_chords_that_hold_super_or_ctrl_alt_and_only_once_asked() {
        let talk = [
            asked("talk", "LOGO+F5"),
            asked("copy", "CTRL+c"),
            asked("lock", "LOGO+l"),
        ];
        let proposed = proposals(&[], "org.example.Talk", &talk);
        let first = decide(
            Vec::new(),
            "org.example.Talk",
            &proposed,
            accepted(&proposed),
        );
        assert!(proposals(&first, "org.example.Talk", &talk).is_empty());
        let rec = [asked("record", "LOGO+F5"), asked("mark", "CTRL+ALT+m")];
        let proposed = proposals(&first, "org.example.Rec", &rec);
        let mut answers = accepted(&proposed);
        answers[0].1 = Trigger::parse("LOGO+F5");
        answers.push(("unasked".to_owned(), Trigger::parse("LOGO+F6")));
        let both = decide(first, "org.example.Rec", &proposed, answers);
        let keys: Vec<(&str, Option<String>)> = both
            .iter()
            .map(|shortcut| (shortcut.id.as_str(), shortcut.trigger.map(Trigger::keys)))
            .collect();
        assert_eq!(
            keys,
            [
                ("talk", Some("Super+F5".to_owned())),
                ("copy", None),
                ("lock", None),
                ("record", None),
                ("mark", Some("Ctrl+Alt+M".to_owned())),
            ]
        );
        assert_eq!(read_global_shortcuts(&write_global_shortcuts(&both)), both);
    }
}
