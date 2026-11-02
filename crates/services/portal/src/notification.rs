use std::collections::HashMap;
use std::fs::File;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_xdg::{Locale, app_name, applications, current_desktops, data_dirs};
use zbus::blocking::Connection;
use zbus::message::Type;
use zbus::zvariant::{OwnedValue, Value};

use crate::OBJECT_PATH;

const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const NOTIFICATIONS_PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.impl.portal.Notification";
const APPLICATION: &str = "org.freedesktop.Application";
const DEFAULT_ACTION: &str = "default";
const EXPORTED: &str = "app.";

type Vardict = HashMap<String, OwnedValue>;
type Key = (String, String);

#[derive(Debug)]
struct Action {
    name: String,
    target: Option<OwnedValue>,
}

#[derive(Debug, Default)]
struct Posted {
    id: u32,
    actions: HashMap<String, Action>,
    picture: Option<PathBuf>,
    token: Option<String>,
}

#[derive(Default)]
struct Live {
    posted: HashMap<Key, Posted>,
    keys: HashMap<u32, Key>,
}

impl Live {
    fn take(&mut self, id: u32) -> Option<Posted> {
        let key = self.keys.remove(&id)?;
        self.posted.remove(&key)
    }

    fn shares_picture(&self, picture: &Path) -> bool {
        self.posted
            .values()
            .any(|posted| posted.picture.as_deref() == Some(picture))
    }
}

#[derive(Clone)]
pub struct Notification {
    session: Connection,
    pictures: Option<PathBuf>,
    live: Arc<Mutex<Live>>,
}

fn value<T: TryFrom<OwnedValue>>(dict: &Vardict, key: &str) -> Option<T> {
    dict.get(key)
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| T::try_from(value).ok())
}

pub fn plain_text(markup: &str) -> String {
    let mut text = String::new();
    let mut rest = markup;
    while let Some(start) = rest.find('<') {
        text.push_str(&rest[..start]);
        rest = rest[start..]
            .find('>')
            .map_or("", |end| &rest[start + end + 1..]);
    }
    text.push_str(rest);
    [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&amp;", "&"),
    ]
    .into_iter()
    .fold(text, |text, (entity, character)| {
        text.replace(entity, character)
    })
}

fn unwrapped(value: OwnedValue) -> OwnedValue {
    if let Value::Value(inner) = &*value
        && let Ok(owned) = inner.try_to_owned()
    {
        return owned;
    }
    value
}

fn urgency(priority: Option<String>) -> u8 {
    match priority.as_deref() {
        Some("low") => 0,
        Some("urgent") => 2,
        _ => 1,
    }
}

fn picture_bytes(kind: &str, data: OwnedValue) -> Option<Vec<u8>> {
    match kind {
        "bytes" => Vec::<u8>::try_from(data).ok(),
        "file-descriptor" => {
            let Value::Fd(fd) = &*data else {
                return None;
            };
            let mut file = File::from(fd.as_fd().try_clone_to_owned().ok()?);
            file.seek(SeekFrom::Start(0)).ok()?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).ok()?;
            Some(bytes)
        }
        _ => None,
    }
}

fn keep_picture(folder: &Path, bytes: &[u8]) -> Option<PathBuf> {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    let svg = bytes.windows(4).take(1024).any(|window| window == b"<svg");
    let extension = if svg { ".svg" } else { "" };
    let path = folder.join(format!("{:016x}{extension}", hasher.finish()));
    if !path.exists() {
        std::fs::create_dir_all(folder)
            .and_then(|()| bande_xdg::replace_file(&path, bytes))
            .inspect_err(|error| tracing::warn!("cannot keep a notification picture: {error}"))
            .ok()?;
    }
    Some(path)
}

struct Outgoing {
    icon: String,
    hints: HashMap<&'static str, Value<'static>>,
    actions: Vec<(String, String, Action)>,
    picture: Option<PathBuf>,
}

impl Notification {
    pub fn serve(session: &Connection) -> zbus::Result<()> {
        let notification = Self {
            session: session.clone(),
            pictures: std::env::var_os("XDG_RUNTIME_DIR")
                .map(|runtime| PathBuf::from(runtime).join("xdg-desktop-portal-bande/pictures")),
            live: Arc::default(),
        };
        notification.follow()?;
        session.object_server().at(OBJECT_PATH, notification)?;
        Ok(())
    }

    fn live(&self) -> MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn follow(&self) -> zbus::Result<()> {
        let rule = zbus::MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(NOTIFICATIONS)?
            .interface(NOTIFICATIONS)?
            .path(NOTIFICATIONS_PATH)?
            .build();
        let signals = zbus::blocking::MessageIterator::for_match_rule(rule, &self.session, None)?;
        let service = self.clone();
        thread::spawn(move || {
            for message in signals.flatten() {
                let header = message.header();
                let body = message.body();
                match header.member().map(|member| member.as_str()) {
                    Some("ActionInvoked") => {
                        if let Ok((id, key)) = body.deserialize::<(u32, String)>() {
                            service.invoked(id, &key);
                        }
                    }
                    Some("ActivationToken") => {
                        if let Ok((id, token)) = body.deserialize::<(u32, String)>() {
                            service.remember_token(id, token);
                        }
                    }
                    Some("NotificationClosed") => {
                        if let Ok((id, _)) = body.deserialize::<(u32, u32)>() {
                            let taken = service.live().take(id);
                            service.forget(taken);
                        }
                    }
                    _ => {}
                }
            }
        });
        Ok(())
    }

    fn remember_token(&self, id: u32, token: String) {
        let mut live = self.live();
        if let Some(key) = live.keys.get(&id).cloned()
            && let Some(posted) = live.posted.get_mut(&key)
        {
            posted.token = Some(token);
        }
    }

    fn forget(&self, posted: Option<Posted>) {
        let Some(picture) = posted.and_then(|posted| posted.picture) else {
            return;
        };
        if !self.live().shares_picture(&picture) {
            let _ = std::fs::remove_file(picture);
        }
    }

    fn invoked(&self, id: u32, key: &str) {
        let found = {
            let mut live = self.live();
            live.keys.get(&id).cloned().and_then(|(app_id, name)| {
                let posted = live.posted.get_mut(&(app_id.clone(), name.clone()))?;
                let action = posted.actions.get(key).map(|action| Action {
                    name: action.name.clone(),
                    target: action
                        .target
                        .as_ref()
                        .and_then(|target| target.try_clone().ok()),
                })?;
                Some((app_id, name, action, posted.token.take()))
            })
        };
        let Some((app_id, name, action, token)) = found else {
            return;
        };
        let platform: HashMap<&str, Value<'_>> = token
            .iter()
            .flat_map(|token| {
                [
                    ("activation-token", Value::from(token.as_str())),
                    ("desktop-startup-id", Value::from(token.as_str())),
                ]
            })
            .collect();
        let target: Vec<Value<'_>> = action
            .target
            .iter()
            .filter_map(|target| target.try_clone().ok().map(Value::from))
            .collect();
        if let Some(exported) = action.name.strip_prefix(EXPORTED)
            && !app_id.is_empty()
        {
            let activated = self.session.call_method(
                Some(app_id.as_str()),
                app_path(&app_id).as_str(),
                Some(APPLICATION),
                "ActivateAction",
                &(exported, &target, &platform),
            );
            match activated {
                Ok(_) => return,
                Err(error) => tracing::warn!("cannot ask {app_id} to {exported}: {error}"),
            }
        }
        let mut parameter = target;
        parameter.push(Value::from(platform));
        let _ = self.session.emit_signal(
            None::<&str>,
            OBJECT_PATH,
            INTERFACE,
            "ActionInvoked",
            &(
                app_id.as_str(),
                name.as_str(),
                action.name.as_str(),
                parameter,
            ),
        );
    }

    fn outgoing(&self, notification: &Vardict, app_id: &str) -> Outgoing {
        let mut hints: HashMap<&'static str, Value<'static>> = HashMap::from([(
            "urgency",
            Value::from(urgency(value(notification, "priority"))),
        )]);
        if !app_id.is_empty() {
            hints.insert("desktop-entry", Value::from(app_id.to_owned()));
        }
        let display: Vec<String> = value(notification, "display-hint").unwrap_or_default();
        for (shown, hint) in [("transient", "transient"), ("persistent", "resident")] {
            if display.iter().any(|given| given == shown) {
                hints.insert(hint, Value::from(true));
            }
        }
        if value::<String>(notification, "sound").as_deref() == Some("silent") {
            hints.insert("suppress-sound", Value::from(true));
        }
        if let Some(category) = value::<String>(notification, "category") {
            hints.insert("category", Value::from(category));
        }
        let mut icon = String::new();
        let mut picture = None;
        if let Some((kind, data)) = value::<(String, OwnedValue)>(notification, "icon") {
            let data = unwrapped(data);
            match kind.as_str() {
                "themed" => {
                    icon = Vec::<String>::try_from(data)
                        .ok()
                        .and_then(|names| names.into_iter().next())
                        .unwrap_or_default();
                }
                _ => {
                    picture = picture_bytes(&kind, data)
                        .and_then(|bytes| keep_picture(self.pictures.as_deref()?, &bytes));
                }
            }
        }
        if let Some(path) = &picture {
            hints.insert(
                "image-path",
                Value::from(path.to_string_lossy().into_owned()),
            );
        }
        let mut actions = Vec::new();
        if let Some(name) = value::<String>(notification, "default-action") {
            actions.push((
                DEFAULT_ACTION.to_owned(),
                String::new(),
                Action {
                    name,
                    target: notification
                        .get("default-action-target")
                        .and_then(|target| target.try_clone().ok()),
                },
            ));
        }
        let buttons: Vec<Vardict> = value(notification, "buttons").unwrap_or_default();
        for (index, button) in buttons.iter().enumerate() {
            let (Some(label), Some(name)) = (
                value::<String>(button, "label"),
                value::<String>(button, "action"),
            ) else {
                continue;
            };
            actions.push((
                format!("button-{index}"),
                label,
                Action {
                    name,
                    target: button
                        .get("target")
                        .and_then(|target| target.try_clone().ok()),
                },
            ));
        }
        Outgoing {
            icon,
            hints,
            actions,
            picture,
        }
    }

    fn add(&self, app_id: String, name: String, notification: &Vardict) -> zbus::fdo::Result<()> {
        let summary: String = value(notification, "title").unwrap_or_default();
        let body = value::<String>(notification, "markup-body")
            .map(|markup| plain_text(&markup))
            .or_else(|| value(notification, "body"))
            .unwrap_or_default();
        let outgoing = self.outgoing(notification, &app_id);
        let key = (app_id.clone(), name);
        let replaces = self.live().posted.get(&key).map_or(0, |posted| posted.id);
        let actions: Vec<&str> = outgoing
            .actions
            .iter()
            .flat_map(|(key, label, _)| [key.as_str(), label.as_str()])
            .collect();
        let entries = applications(
            &data_dirs(),
            Locale::from_env().as_ref(),
            &current_desktops(),
        );
        let id: u32 = self
            .session
            .call_method(
                Some(NOTIFICATIONS),
                NOTIFICATIONS_PATH,
                Some(NOTIFICATIONS),
                "Notify",
                &(
                    app_name(&entries, &app_id),
                    replaces,
                    outgoing.icon.as_str(),
                    summary,
                    body,
                    actions,
                    &outgoing.hints,
                    -1i32,
                ),
            )?
            .body()
            .deserialize()?;
        let replaced = {
            let mut live = self.live();
            let previous = live.posted.insert(
                key.clone(),
                Posted {
                    id,
                    actions: outgoing
                        .actions
                        .into_iter()
                        .map(|(key, _, action)| (key, action))
                        .collect(),
                    picture: outgoing.picture,
                    token: None,
                },
            );
            live.keys.insert(id, key);
            previous.filter(|previous| previous.id != id)
        };
        if let Some(previous) = &replaced {
            self.live().keys.remove(&previous.id);
        }
        self.forget(replaced);
        Ok(())
    }

    fn remove(&self, app_id: String, name: String) {
        let taken = {
            let mut live = self.live();
            let taken = live.posted.remove(&(app_id, name));
            if let Some(posted) = &taken {
                live.keys.remove(&posted.id);
            }
            taken
        };
        if let Some(posted) = &taken {
            let _ = self.session.call_method(
                Some(NOTIFICATIONS),
                NOTIFICATIONS_PATH,
                Some(NOTIFICATIONS),
                "CloseNotification",
                &posted.id,
            );
        }
        self.forget(taken);
    }
}

fn app_path(app_id: &str) -> String {
    format!("/{}", app_id.replace('.', "/").replace('-', "_"))
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Notification")]
impl Notification {
    async fn add_notification(
        &self,
        app_id: String,
        id: String,
        notification: Vardict,
    ) -> zbus::fdo::Result<()> {
        let service = self.clone();
        blocking::unblock(move || service.add(app_id, id, &notification)).await
    }

    async fn remove_notification(&self, app_id: String, id: String) {
        let service = self.clone();
        blocking::unblock(move || service.remove(app_id, id)).await;
    }

    #[zbus(property)]
    fn supported_options(&self) -> HashMap<String, OwnedValue> {
        HashMap::new()
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        2
    }
}
