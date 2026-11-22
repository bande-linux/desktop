use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_keyboard::{GlobalShortcut, Trigger, read_global_shortcuts, write_global_shortcuts};
use zbus::MatchRule;
use zbus::message::Type as MessageType;
use zbus::object_server::{ObjectServer, SignalEmitter};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::file_chooser::option;
use crate::request::{OTHER, SUCCESS};
use crate::{OBJECT_PATH, from_frontend};

pub const COMPOSITOR_NAME: &str = "org.bande.GlobalShortcuts";
pub const COMPOSITOR_PATH: &str = "/org/bande/GlobalShortcuts";
const INTERFACE: &str = "org.freedesktop.impl.portal.GlobalShortcuts";
const RELAYED: [&str; 2] = ["Activated", "Deactivated"];

type Options = HashMap<String, OwnedValue>;
type Described = (String, HashMap<String, OwnedValue>);
type Sessions = Arc<Mutex<HashMap<OwnedObjectPath, String>>>;

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

pub fn bind(
    mut known: Vec<GlobalShortcut>,
    app_id: &str,
    requested: Vec<(String, String, Option<Trigger>)>,
) -> Vec<GlobalShortcut> {
    for (id, description, preferred) in requested {
        if let Some(bound) = known
            .iter_mut()
            .find(|shortcut| shortcut.app_id == app_id && shortcut.id == id)
        {
            bound.description = description;
            continue;
        }
        let trigger = preferred.filter(|trigger| {
            trigger.assignable()
                && !known
                    .iter()
                    .any(|shortcut| shortcut.trigger == Some(*trigger))
        });
        known.push(GlobalShortcut {
            app_id: app_id.to_owned(),
            id,
            description,
            trigger,
        });
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
pub struct GlobalShortcuts {
    sessions: Sessions,
    file: Option<PathBuf>,
    writing: Arc<Mutex<()>>,
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
        let _ = server.remove::<Self, _>(&self.path).await;
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
        let listening: Vec<OwnedObjectPath> = lock(sessions)
            .iter()
            .filter(|(_, owner)| **owner == app_id)
            .map(|(path, _)| path.clone())
            .collect();
        for path in listening {
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

impl GlobalShortcuts {
    pub fn serve(session: &zbus::blocking::Connection) -> zbus::Result<()> {
        let shortcuts = Self {
            sessions: Sessions::default(),
            file: bande_xdg::global_shortcuts_file(),
            writing: Arc::default(),
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

    async fn update(
        &self,
        change: impl FnOnce(Vec<GlobalShortcut>) -> Vec<GlobalShortcut> + Send + 'static,
    ) -> io::Result<Vec<GlobalShortcut>> {
        let (file, writing) = (self.file.clone(), Arc::clone(&self.writing));
        blocking::unblock(move || {
            let Some(file) = file else {
                return Err(io::Error::other("there is no state folder"));
            };
            let _held = lock(&writing);
            let before = read_file(&file)?;
            let after = change(before.clone());
            if after != before {
                if let Some(folder) = file.parent() {
                    std::fs::create_dir_all(folder)?;
                }
                bande_xdg::replace_file(&file, write_global_shortcuts(&after))?;
            }
            Ok(after)
        })
        .await
    }

    fn app_of(&self, session: &ObjectPath<'_>) -> Option<String> {
        lock(&self.sessions)
            .get(&OwnedObjectPath::from(session.to_owned()))
            .cloned()
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
        _handle: ObjectPath<'_>,
        session_handle: ObjectPath<'_>,
        shortcuts: Vec<Described>,
        _parent_window: String,
        _options: Options,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> (u32, HashMap<String, OwnedValue>) {
        let caller = from_frontend(connection, &header).await;
        let Some(app_id) = self.app_of(&session_handle).filter(|_| caller) else {
            return (OTHER, HashMap::new());
        };
        let requested: Vec<(String, String, Option<Trigger>)> = shortcuts
            .iter()
            .map(|(id, details)| {
                let description: String = option(details, "description").unwrap_or_default();
                let preferred: Option<String> = option(details, "preferred_trigger");
                (
                    id.clone(),
                    description,
                    preferred.as_deref().and_then(Trigger::parse),
                )
            })
            .collect();
        let ids: Vec<String> = requested.iter().map(|(id, ..)| id.clone()).collect();
        let owner = app_id.clone();
        match self
            .update(move |known| bind(known, &owner, requested))
            .await
        {
            Ok(all) => (
                SUCCESS,
                results(
                    all.iter()
                        .filter(|shortcut| shortcut.app_id == app_id && ids.contains(&shortcut.id))
                        .map(described)
                        .collect(),
                ),
            ),
            Err(error) => {
                tracing::warn!("cannot keep the shortcuts of {app_id}: {error}");
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
        match self.update(|known| known).await {
            Ok(all) => (
                SUCCESS,
                results(
                    all.iter()
                        .filter(|shortcut| shortcut.app_id == app_id)
                        .map(described)
                        .collect(),
                ),
            ),
            Err(error) => {
                tracing::warn!("cannot read the shortcuts of {app_id}: {error}");
                (OTHER, HashMap::new())
            }
        }
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(id: &str, trigger: &str) -> (String, String, Option<Trigger>) {
        (id.to_owned(), String::new(), Trigger::parse(trigger))
    }

    #[test]
    fn an_app_gets_only_free_chords_that_hold_super_or_ctrl_alt() {
        let first = bind(
            Vec::new(),
            "org.example.Talk",
            vec![
                asked("talk", "LOGO+F5"),
                asked("copy", "CTRL+c"),
                asked("lock", "LOGO+l"),
            ],
        );
        let both = bind(
            first,
            "org.example.Rec",
            vec![asked("record", "LOGO+F5"), asked("mark", "CTRL+ALT+m")],
        );
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
