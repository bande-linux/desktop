use std::collections::HashMap;
use std::fmt;
use std::sync::mpsc::{self, Sender};

use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, SearchField, SecureField, Stack, Widget};
use bande_xdg::{
    DesktopEntry, Locale, app_for_identity, app_name, current_desktops, data_dirs, handlers,
};
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zeroize::Zeroizing;

use crate::L10N;
use crate::failure::unexpected;
use crate::follow::{Command, Update, Updates, spawn};
use crate::forget::Forget;
use crate::pages::{FIELD_SIZE, Widgets, boxed, muted_row, submit};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const SECRETS: &str = "org.freedesktop.secrets";
const SECRETS_PATH: &str = "/org/freedesktop/secrets";
const SERVICE: &str = "org.freedesktop.Secret.Service";
const ITEM: &str = "org.freedesktop.Secret.Item";
const LOGIN: &str = "org.freedesktop.login1";
const SESSION: &str = "org.freedesktop.login1.Session";
const CREATOR: &str = "bande:creator";
const NETWORK_SCHEMA: &str = "org.freedesktop.NetworkManager.Connection";
const ACCOUNT_KEYS: [&str; 6] = ["username", "user", "account", "email", "login", "server"];

#[derive(Clone, PartialEq, Eq)]
pub struct Concealed(pub Zeroizing<String>);

impl fmt::Debug for Concealed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Concealed")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub path: String,
    pub label: String,
    pub account: Option<String>,
    pub app: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordCommand {
    Unlock,
    Lock,
    Refresh,
    Reveal(String),
    Copy(String),
    Change(String, Concealed),
    Delete(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keychain {
    pub unlocked: bool,
    pub items: Vec<Saved>,
    pub query: String,
    pub revealed: Option<(String, Concealed)>,
    pub changing: Option<String>,
}

type WireSecret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

fn app_of(entries: &[DesktopEntry], attributes: &HashMap<String, String>) -> String {
    if let Some(creator) = attributes.get(CREATOR) {
        return match app_for_identity(entries, creator) {
            Some(entry) => entry.name.clone(),
            None => creator
                .strip_prefix("flatpak:")
                .or_else(|| creator.rsplit('/').next())
                .unwrap_or(creator)
                .to_owned(),
        };
    }
    if attributes.get("xdg:schema").map(String::as_str) == Some(NETWORK_SCHEMA) {
        return L10N.tr("password-network");
    }
    attributes
        .get("application")
        .or_else(|| attributes.get("xdg:schema"))
        .map(|app| app_name(entries, app))
        .unwrap_or_else(|| L10N.tr("password-unknown-app"))
}

struct Client {
    session: Connection,
    secrets: OwnedObjectPath,
}

impl Client {
    fn open(session: &Connection) -> zbus::Result<Self> {
        let (_, secrets): (OwnedValue, OwnedObjectPath) = session
            .call_method(
                Some(SECRETS),
                SECRETS_PATH,
                Some(SERVICE),
                "OpenSession",
                &("plain", Value::from("")),
            )?
            .body()
            .deserialize()?;
        Ok(Self {
            session: session.clone(),
            secrets,
        })
    }

    fn item(&self, path: &str) -> zbus::Result<Proxy<'_>> {
        Proxy::new(&self.session, SECRETS, path.to_owned(), ITEM)
    }

    fn reveal(&self) -> zbus::Result<bool> {
        let prompt: OwnedObjectPath = self
            .session
            .call_method(
                Some(SECRETS),
                SECRETS_PATH,
                Some("org.bande.Secrets"),
                "Reveal",
                &(),
            )?
            .body()
            .deserialize()?;
        let proxy = Proxy::new(
            &self.session,
            SECRETS,
            prompt,
            "org.freedesktop.Secret.Prompt",
        )?;
        let mut completed = proxy.receive_signal("Completed")?;
        proxy.call_method("Prompt", &(""))?;
        let Some(signal) = completed.next() else {
            return Ok(false);
        };
        let (dismissed, _): (bool, OwnedValue) = signal.body().deserialize()?;
        Ok(!dismissed)
    }

    fn items(&self) -> zbus::Result<Vec<Saved>> {
        let entries = handlers(
            &data_dirs(),
            Locale::from_env().as_ref(),
            &current_desktops(),
        );
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = self
            .session
            .call_method(
                Some(SECRETS),
                SECRETS_PATH,
                Some(SERVICE),
                "SearchItems",
                &(HashMap::<String, String>::new(),),
            )?
            .body()
            .deserialize()?;
        let mut items = Vec::new();
        for path in unlocked.into_iter().chain(locked) {
            let item = self.item(path.as_str())?;
            let attributes: HashMap<String, String> = item.get_property("Attributes")?;
            let label: String = item.get_property("Label")?;
            items.push(Saved {
                path: path.to_string(),
                account: ACCOUNT_KEYS
                    .iter()
                    .find_map(|key| attributes.get(*key).cloned()),
                app: app_of(&entries, &attributes),
                label,
            });
        }
        items.sort_by_key(|item| item.label.to_lowercase());
        Ok(items)
    }

    fn secret(&self, path: &str) -> zbus::Result<Concealed> {
        let (_, _, value, _): WireSecret = self
            .item(path)?
            .call_method("GetSecret", &(&self.secrets,))?
            .body()
            .deserialize()?;
        let value = Zeroizing::new(value);
        Ok(Concealed(Zeroizing::new(
            String::from_utf8_lossy(&value).into_owned(),
        )))
    }

    fn change(&self, path: &str, password: &Concealed) -> zbus::Result<()> {
        let secret = (
            &self.secrets,
            Vec::<u8>::new(),
            password.0.as_bytes().to_vec(),
            "text/plain",
        );
        self.item(path)?.call_method("SetSecret", &(secret,))?;
        Ok(())
    }

    fn delete(&self, path: &str) -> zbus::Result<()> {
        self.item(path)?.call_method("Delete", &())?;
        Ok(())
    }
}

fn follow_lock(system: &Connection, commands: Sender<PasswordCommand>) {
    let system = system.clone();
    spawn("keychain-lock", move || {
        let follow = || -> zbus::Result<()> {
            let rule = MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(LOGIN)?
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?
                .add_arg(SESSION)?
                .build();
            let changes = MessageIterator::for_match_rule(rule, &system, None)?;
            for message in changes {
                let Ok(message) = message else {
                    continue;
                };
                let locked = message
                    .body()
                    .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                    .ok()
                    .and_then(|(_, changed, _)| bool::try_from(changed.get("LockedHint")?).ok())
                    .unwrap_or(false);
                if locked && commands.send(PasswordCommand::Lock).is_err() {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the lock screen: {error}");
        }
    });
}

fn follow_items(session: &Connection, commands: Sender<PasswordCommand>) {
    let session = session.clone();
    spawn("keychain-items", move || {
        let follow = || -> zbus::Result<()> {
            let rule = MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(SECRETS)?
                .interface("org.freedesktop.Secret.Collection")?
                .build();
            for _ in MessageIterator::for_match_rule(rule, &session, None)? {
                if commands.send(PasswordCommand::Refresh).is_err() {
                    break;
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the keychain: {error}");
        }
    });
}

pub fn follow_passwords(
    session: &Connection,
    system: Option<&Connection>,
    updates: Updates,
) -> Sender<PasswordCommand> {
    let (sender, receiver) = mpsc::channel();
    follow_items(session, sender.clone());
    if let Some(system) = system {
        follow_lock(system, sender.clone());
    }
    let session = session.clone();
    spawn("passwords", move || {
        let mut client: Option<Client> = None;
        let mut unlocked = false;
        while let Ok(command) = receiver.recv() {
            if command == PasswordCommand::Lock {
                client = None;
                unlocked = false;
                if !updates.send(Update::Passwords(false, Vec::new())) {
                    break;
                }
                continue;
            }
            if command == PasswordCommand::Refresh && !unlocked {
                continue;
            }
            let open = match client.take() {
                Some(open) => Ok(open),
                None => Client::open(&session),
            };
            let open = match open {
                Ok(open) => open,
                Err(error) => {
                    updates.send(unexpected(error));
                    continue;
                }
            };
            let result = match command {
                PasswordCommand::Unlock => open.reveal().map(|revealed| unlocked = revealed),
                PasswordCommand::Reveal(path) => open
                    .secret(&path)
                    .map(|secret| updates.send(Update::RevealedPassword(path, secret)))
                    .map(drop),
                PasswordCommand::Copy(path) => open
                    .secret(&path)
                    .map(|secret| updates.send(Update::CopiedPassword(secret)))
                    .map(drop),
                PasswordCommand::Change(path, password) => open.change(&path, &password),
                PasswordCommand::Delete(path) => open.delete(&path),
                PasswordCommand::Refresh | PasswordCommand::Lock => Ok(()),
            };
            if let Err(error) = result {
                updates.send(unexpected(error));
            }
            let items = match unlocked {
                true => open.items().unwrap_or_else(|error| {
                    updates.send(unexpected(error));
                    Vec::new()
                }),
                false => Vec::new(),
            };
            client = Some(open);
            if !updates.send(Update::Passwords(unlocked, items)) {
                break;
            }
        }
    });
    sender
}

fn matches(item: &Saved, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || [Some(&item.label), item.account.as_ref(), Some(&item.app)]
            .into_iter()
            .flatten()
            .any(|text| text.to_lowercase().contains(&query))
}

impl SettingsView {
    pub(crate) fn take_passwords(&mut self, unlocked: bool, items: Vec<Saved>) {
        if !unlocked {
            self.keychain = Keychain::default();
            return;
        }
        let known = |path: &Option<String>| {
            path.as_ref()
                .is_some_and(|path| items.iter().any(|item| item.path == *path))
        };
        if !known(
            &self
                .keychain
                .revealed
                .as_ref()
                .map(|(path, _)| path.clone()),
        ) {
            self.keychain.revealed = None;
        }
        if !known(&self.keychain.changing) {
            self.keychain.changing = None;
        }
        self.keychain.unlocked = true;
        self.keychain.items = items;
    }

    pub(crate) fn password_command(&mut self, command: PasswordCommand) {
        match &command {
            PasswordCommand::Reveal(path) => {
                if self
                    .keychain
                    .revealed
                    .as_ref()
                    .is_some_and(|(shown, _)| shown == path)
                {
                    self.keychain.revealed = None;
                    return;
                }
            }
            PasswordCommand::Lock => self.keychain = Keychain::default(),
            PasswordCommand::Change(..) => self.keychain.changing = None,
            _ => {}
        }
        if let PasswordCommand::Reveal(path) | PasswordCommand::Copy(path) = &command {
            self.focus_next = Some(Anchor::Item(format!("password:{path}")));
        }
        self.commands.push(Command::Passwords(command));
    }

    pub(crate) fn keychain_rows(&mut self) -> Widgets {
        if !self.keychain.unlocked {
            let mut unlock = intent(&self.intents, |()| {
                Intent::Passwords(PasswordCommand::Unlock)
            });
            let button = Button::new(&L10N.tr("passwords-unlock"), move || unlock(()));
            self.anchors
                .push((Anchor::Item("passwords-unlock".to_owned()), button.id()));
            return vec![boxed(
                Row::new(
                    Text::new(&L10N.tr("passwords-locked"), Tone::Primary),
                    Some(boxed(button)),
                )
                .detail(Text::new(&L10N.tr("passwords-locked-detail"), Tone::Muted).caption()),
            )];
        }
        let field = SearchField::new(
            &L10N.tr("passwords-search"),
            &self.keychain.query,
            submit(&self.intents, Intent::PasswordQuery),
        );
        self.anchors.push((Anchor::Picker, field.id()));
        let mut lock = intent(&self.intents, |()| Intent::Passwords(PasswordCommand::Lock));
        let mut rows = vec![boxed(Row::new(
            Text::new(&L10N.tr("passwords-unlocked"), Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&L10N.tr("passwords-lock"), move || lock(()))),
                ],
            ))),
        ))];
        let items = self
            .keychain
            .items
            .iter()
            .filter(|item| matches(item, &self.keychain.query))
            .cloned()
            .collect::<Vec<_>>();
        if items.is_empty() {
            rows.push(muted_row(&L10N.tr(
                if self.keychain.query.trim().is_empty() {
                    "passwords-none"
                } else {
                    "passwords-no-match"
                },
            )));
        }
        for item in items {
            rows.push(self.keychain_row(item));
        }
        rows
    }

    fn keychain_row(&mut self, item: Saved) -> Box<dyn Widget> {
        let target = Forget::Password(item.path.clone());
        if self.forgetting.as_ref() == Some(&target) {
            return self.confirm_row(
                &L10N.tr_args(
                    "password-delete-question",
                    [("name", item.label.clone().into())],
                ),
                &L10N.tr("password-delete-warning"),
                &L10N.tr("password-delete"),
            );
        }
        let anchor = format!("password:{}", item.path);
        if self.keychain.changing.as_deref() == Some(item.path.as_str()) {
            let path = item.path.clone();
            let mut change = intent(&self.intents, move |password: String| {
                Intent::Passwords(PasswordCommand::Change(
                    path.clone(),
                    Concealed(Zeroizing::new(password)),
                ))
            });
            let field = SecureField::new(&L10N.tr("password-new"), FIELD_SIZE, move |password| {
                change(password.to_owned())
            });
            self.anchors.push((Anchor::Password, field.id()));
            let mut cancel = intent(&self.intents, |()| Intent::ChangePassword(None));
            let controls = Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&L10N.tr("cancel"), move || cancel(())).cancel()),
                ],
            );
            return boxed(
                Row::new(Text::new(&item.label, Tone::Primary), Some(boxed(controls)))
                    .highlighted(true),
            );
        }
        let revealed = self
            .keychain
            .revealed
            .as_ref()
            .filter(|(path, _)| *path == item.path)
            .map(|(_, secret)| secret.0.to_string());
        let detail = match (&revealed, &item.account) {
            (Some(secret), _) => secret.clone(),
            (None, Some(account)) => format!("{account} · {}", item.app),
            (None, None) => item.app.clone(),
        };
        let button = |label: &str, act: Box<dyn FnMut()>| boxed(Button::new(label, act));
        let (reveal, copy, edit) = (item.path.clone(), item.path.clone(), item.path.clone());
        let mut show = intent(&self.intents, move |()| {
            Intent::Passwords(PasswordCommand::Reveal(reveal.clone()))
        });
        let mut copied = intent(&self.intents, move |()| {
            Intent::Passwords(PasswordCommand::Copy(copy.clone()))
        });
        let mut changing = intent(&self.intents, move |()| {
            Intent::ChangePassword(Some(edit.clone()))
        });
        let controls = Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            vec![
                button(
                    &L10N.tr(if revealed.is_some() {
                        "password-hide"
                    } else {
                        "password-show"
                    }),
                    Box::new(move || show(())),
                ),
                button(&L10N.tr("password-copy"), Box::new(move || copied(()))),
                button(&L10N.tr("password-change"), Box::new(move || changing(()))),
                {
                    let mut ask = intent(&self.intents, move |()| {
                        Intent::AskForget(Some(target.clone()))
                    });
                    button(
                        &L10N.tr("password-delete-ellipsis"),
                        Box::new(move || ask(())),
                    )
                },
            ],
        );
        let row = Row::new(Text::new(&item.label, Tone::Primary), Some(boxed(controls)))
            .detail(Text::new(&detail, Tone::Muted).caption());
        self.item_row(anchor, row)
    }
}
