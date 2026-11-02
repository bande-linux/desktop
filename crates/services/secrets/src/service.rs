use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use async_lock::Mutex as AsyncMutex;
use futures_lite::StreamExt;
use oo7::Secret;
use oo7::file::UnlockedItem;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, interface};

use crate::dialog::{Dialog, Kind, Reply, Request};
use crate::peer;
use crate::session::{self, Session};
use crate::store::{self, ALLOWED, CREATOR, Keychain, LOGIN, SESSION};

pub const BUS_NAME: &str = "org.freedesktop.secrets";
pub const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const COLLECTIONS: &str = "/org/freedesktop/secrets/collection/";
const ALIASES: &str = "/org/freedesktop/secrets/aliases/";
const NO_PROMPT: &str = "/";
const ITEM_LABEL: &str = "org.freedesktop.Secret.Item.Label";
const ITEM_ATTRIBUTES: &str = "org.freedesktop.Secret.Item.Attributes";
const COLLECTION_LABEL: &str = "org.freedesktop.Secret.Collection.Label";

pub type WireSecret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.Secret.Error")]
pub enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    IsLocked(String),
    NoSession(String),
    NoSuchObject(String),
}

fn fdo(error: zbus::fdo::Error) -> Error {
    Error::ZBus(zbus::Error::FDO(Box::new(error)))
}

fn failed(message: impl ToString) -> Error {
    fdo(zbus::fdo::Error::Failed(message.to_string()))
}

impl From<store::Error> for Error {
    fn from(error: store::Error) -> Self {
        match error {
            store::Error::Locked => Self::IsLocked("the keychain is locked".to_owned()),
            store::Error::NoSuchCollection | store::Error::NoSuchItem => {
                Self::NoSuchObject(error.to_string())
            }
            store::Error::Protected => fdo(zbus::fdo::Error::NotSupported(error.to_string())),
            other => failed(other),
        }
    }
}

impl From<session::Error> for Error {
    fn from(error: session::Error) -> Self {
        match error {
            session::Error::Unsupported => fdo(zbus::fdo::Error::NotSupported(
                "unsupported session algorithm".to_owned(),
            )),
            session::Error::BadInput => {
                fdo(zbus::fdo::Error::InvalidArgs("malformed secret".to_owned()))
            }
            session::Error::Crypto => failed("session encryption failed"),
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn object_path(text: String) -> OwnedObjectPath {
    OwnedObjectPath::try_from(text).unwrap_or_else(|_| no_prompt())
}

fn no_prompt() -> OwnedObjectPath {
    OwnedObjectPath::from(ObjectPath::from_static_str_unchecked(NO_PROMPT))
}

pub fn collection_path(name: &str) -> OwnedObjectPath {
    object_path(format!("{COLLECTIONS}{name}"))
}

pub fn item_path(name: &str, id: u64) -> OwnedObjectPath {
    object_path(format!("{COLLECTIONS}{name}/{id}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Collection(String),
    Item(String, u64),
}

fn target(keychain: &Keychain, path: &str) -> Option<Target> {
    let (collection, rest) = if let Some(rest) = path.strip_prefix(COLLECTIONS) {
        let (name, rest) = rest.split_once('/').unwrap_or((rest, ""));
        (name.to_owned(), rest)
    } else {
        let rest = path.strip_prefix(ALIASES)?;
        let (alias, rest) = rest.split_once('/').unwrap_or((rest, ""));
        let name = match alias {
            store::DEFAULT => keychain.default_collection().to_owned(),
            SESSION => SESSION.to_owned(),
            _ => return None,
        };
        (name, rest)
    };
    keychain.collection(&collection)?;
    if rest.is_empty() {
        Some(Target::Collection(collection))
    } else {
        Some(Target::Item(collection, rest.parse().ok()?))
    }
}

pub struct Caller {
    pub sender: String,
    pub identity: Option<String>,
}

#[derive(Default)]
struct Peer {
    identity: Option<Option<String>>,
    granted: HashSet<OwnedObjectPath>,
    declined: bool,
}

enum Job {
    Unlock(Vec<OwnedObjectPath>),
    Create { label: String, alias: String },
}

impl Job {
    fn refusal(&self) -> OwnedValue {
        let value = match self {
            Self::Unlock(_) => Value::from(Vec::<OwnedObjectPath>::new()),
            Self::Create { .. } => Value::from(no_prompt()),
        };
        value
            .try_into_owned()
            .unwrap_or_else(|_| OwnedValue::from(false))
    }
}

struct Pending {
    owner: String,
    job: Option<Job>,
    task: Option<zbus::Task<()>>,
    refusal: OwnedValue,
}

pub struct Daemon {
    keychain: AsyncMutex<Keychain>,
    searching: AsyncMutex<()>,
    registered: AsyncMutex<HashSet<OwnedObjectPath>>,
    sessions: Mutex<HashMap<OwnedObjectPath, Session>>,
    peers: Mutex<HashMap<String, Peer>>,
    prompts: Mutex<HashMap<OwnedObjectPath, Pending>>,
    login_attempt: Mutex<Option<Secret>>,
    counter: AtomicU64,
    connection: OnceLock<Connection>,
    prompter: PathBuf,
}

fn allowed_list(item: &UnlockedItem) -> Vec<&str> {
    item.attributes()
        .get(ALLOWED)
        .map(|list| list.lines().collect())
        .unwrap_or_default()
}

fn open_to(item: &UnlockedItem, identity: Option<&str>) -> bool {
    match item.attributes().get(CREATOR) {
        None => true,
        Some(creator) => identity
            .is_some_and(|identity| identity == creator || allowed_list(item).contains(&identity)),
    }
}

fn client_attributes(attributes: HashMap<String, String>) -> HashMap<String, String> {
    attributes
        .into_iter()
        .filter(|(key, _)| !key.starts_with("bande:"))
        .collect()
}

fn keep_internal(
    stored: &HashMap<String, String>,
    mut wanted: HashMap<String, String>,
) -> HashMap<String, String> {
    for (key, value) in stored {
        if store::internal(key) && !wanted.contains_key(key) {
            wanted.insert(key.clone(), value.clone());
        }
    }
    wanted
}

impl Daemon {
    pub fn new(keychain: Keychain, prompter: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            keychain: AsyncMutex::new(keychain),
            searching: AsyncMutex::new(()),
            registered: AsyncMutex::new(HashSet::new()),
            sessions: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            prompts: Mutex::new(HashMap::new()),
            login_attempt: Mutex::new(None),
            counter: AtomicU64::new(1),
            connection: OnceLock::new(),
            prompter,
        })
    }

    fn connection(&self) -> Result<&Connection> {
        self.connection
            .get()
            .ok_or_else(|| failed("the keychain is not on the bus yet"))
    }

    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn serve(self: &Arc<Self>, connection: &Connection) -> zbus::Result<()> {
        if self.connection.set(connection.clone()).is_err() {
            return Err(zbus::Error::Failure(
                "the keychain is already serving".to_owned(),
            ));
        }
        let server = connection.object_server();
        server
            .at(
                SERVICE_PATH,
                ServiceObject {
                    daemon: self.clone(),
                },
            )
            .await?;
        let names = self
            .keychain
            .lock()
            .await
            .collections()
            .iter()
            .map(|collection| collection.name.clone())
            .collect::<Vec<_>>();
        for name in names {
            self.register_collection(&name).await?;
        }
        self.register_aliases().await?;
        self.sync().await;
        self.watch_peers(connection).await?;
        connection
            .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
            .await?;
        Ok(())
    }

    async fn register_collection(self: &Arc<Self>, name: &str) -> zbus::Result<()> {
        self.connection()
            .map_err(|_| zbus::Error::Failure("not connected".to_owned()))?
            .object_server()
            .at(
                collection_path(name),
                CollectionObject {
                    daemon: self.clone(),
                    name: name.to_owned(),
                },
            )
            .await?;
        Ok(())
    }

    async fn register_aliases(self: &Arc<Self>) -> zbus::Result<()> {
        let default = self.keychain.lock().await.default_collection().to_owned();
        let server = self
            .connection()
            .map_err(|_| zbus::Error::Failure("not connected".to_owned()))?
            .object_server();
        for (alias, name) in [(store::DEFAULT, default.as_str()), (SESSION, SESSION)] {
            let path = object_path(format!("{ALIASES}{alias}"));
            let _ = server.remove::<CollectionObject, _>(&path).await;
            server
                .at(
                    path,
                    CollectionObject {
                        daemon: self.clone(),
                        name: name.to_owned(),
                    },
                )
                .await?;
        }
        Ok(())
    }

    async fn sync(self: &Arc<Self>) {
        let Ok(connection) = self.connection() else {
            return;
        };
        let wanted = {
            let keychain = self.keychain.lock().await;
            keychain
                .collections()
                .iter()
                .flat_map(|collection| {
                    collection
                        .entries()
                        .map(|entry| {
                            (
                                item_path(&collection.name, entry.id),
                                collection.name.clone(),
                                entry.id,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        let server = connection.object_server();
        let mut registered = self.registered.lock().await;
        let wanted_paths = wanted
            .iter()
            .map(|(path, _, _)| path.clone())
            .collect::<HashSet<_>>();
        for stale in registered
            .difference(&wanted_paths)
            .cloned()
            .collect::<Vec<_>>()
        {
            if let Err(error) = server.remove::<ItemObject, _>(&stale).await {
                tracing::warn!("cannot remove an item object: {error}");
            }
            registered.remove(&stale);
        }
        for (path, collection, id) in wanted {
            if registered.contains(&path) {
                continue;
            }
            let item = ItemObject {
                daemon: self.clone(),
                collection,
                id,
            };
            match server.at(&path, item).await {
                Ok(_) => {
                    registered.insert(path);
                }
                Err(error) => tracing::warn!("cannot add an item object: {error}"),
            }
        }
    }

    async fn watch_peers(self: &Arc<Self>, connection: &Connection) -> zbus::Result<()> {
        let bus = zbus::fdo::DBusProxy::new(connection).await?;
        let mut changes = bus.receive_name_owner_changed().await?;
        let daemon = Arc::downgrade(self);
        connection
            .executor()
            .spawn(
                async move {
                    while let Some(change) = changes.next().await {
                        let Ok(arguments) = change.args() else {
                            continue;
                        };
                        let name = arguments.name().as_str();
                        if !name.starts_with(':') || arguments.new_owner().is_some() {
                            continue;
                        }
                        let Some(daemon) = daemon.upgrade() else {
                            break;
                        };
                        daemon.forget_peer(name).await;
                    }
                },
                "bande-secrets peers",
            )
            .detach();
        Ok(())
    }

    async fn forget_peer(self: &Arc<Self>, name: &str) {
        guard(&self.peers).remove(name);
        let sessions = {
            let mut sessions = guard(&self.sessions);
            let closed = sessions
                .iter()
                .filter(|(_, session)| session.owner == name)
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            for path in &closed {
                sessions.remove(path);
            }
            closed
        };
        let prompts = {
            let mut prompts = guard(&self.prompts);
            let owned = prompts
                .iter()
                .filter(|(_, pending)| pending.owner == name)
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>();
            for path in &owned {
                prompts.remove(path);
            }
            owned
        };
        if let Ok(connection) = self.connection() {
            let server = connection.object_server();
            for path in sessions {
                let _ = server.remove::<SessionObject, _>(&path).await;
            }
            for path in prompts {
                let _ = server.remove::<PromptObject, _>(&path).await;
            }
        }
    }

    pub async fn caller(&self, header: &Header<'_>) -> Caller {
        let sender = header
            .sender()
            .map(|sender| sender.as_str().to_owned())
            .unwrap_or_default();
        let cached = guard(&self.peers)
            .get(&sender)
            .and_then(|peer| peer.identity.clone());
        let identity = match cached {
            Some(identity) => identity,
            None => {
                let identity = match self.connection() {
                    Ok(connection) => peer::identity(connection, &sender).await,
                    Err(_) => None,
                };
                guard(&self.peers)
                    .entry(sender.clone())
                    .or_default()
                    .identity = Some(identity.clone());
                identity
            }
        };
        Caller { sender, identity }
    }

    fn may_use(&self, caller: &Caller, path: &OwnedObjectPath, item: &UnlockedItem) -> bool {
        open_to(item, caller.identity.as_deref())
            || guard(&self.peers)
                .get(&caller.sender)
                .is_some_and(|peer| peer.granted.contains(path))
    }

    fn grant(&self, sender: &str, paths: impl IntoIterator<Item = OwnedObjectPath>) {
        guard(&self.peers)
            .entry(sender.to_owned())
            .or_default()
            .granted
            .extend(paths);
    }

    pub fn revoke_grants(&self) {
        for peer in guard(&self.peers).values_mut() {
            peer.granted.clear();
        }
    }

    async fn locked_changed(&self, name: &str) {
        let Ok(connection) = self.connection() else {
            return;
        };
        let server = connection.object_server();
        let mut paths = vec![collection_path(name)];
        let default = self.keychain.lock().await.default_collection().to_owned();
        if default == name {
            paths.push(object_path(format!("{ALIASES}{}", store::DEFAULT)));
        }
        for path in paths {
            if let Ok(interface) = server.interface::<_, CollectionObject>(&path).await {
                let _ = interface
                    .get()
                    .await
                    .locked_changed(interface.signal_emitter())
                    .await;
            }
        }
    }

    pub async fn login(self: &Arc<Self>, password: Secret) -> bool {
        let unlocked = self
            .keychain
            .lock()
            .await
            .unlock(LOGIN, password.clone())
            .await;
        match unlocked {
            Ok(()) => {
                *guard(&self.login_attempt) = None;
                self.sync().await;
                self.locked_changed(LOGIN).await;
                true
            }
            Err(store::Error::WrongPassword) => {
                tracing::info!("the login password does not open the login keychain");
                *guard(&self.login_attempt) = Some(password);
                false
            }
            Err(error) => {
                tracing::warn!("cannot open the login keychain: {error}");
                false
            }
        }
    }

    pub async fn change_login(self: &Arc<Self>, old: Secret, new: Secret) -> bool {
        let locked = self
            .keychain
            .lock()
            .await
            .collection(LOGIN)
            .is_none_or(store::Collection::locked);
        if locked && !self.login(old).await && !self.login(new.clone()).await {
            return false;
        }
        let changed = self.keychain.lock().await.change_password(LOGIN, new).await;
        match changed {
            Ok(()) => {
                self.sync().await;
                true
            }
            Err(error) => {
                tracing::warn!("cannot change the login keychain password: {error}");
                false
            }
        }
    }

    async fn dialog(&self, request: &Request) -> Option<Dialog> {
        let connection = self.connection().ok()?;
        match Dialog::open(&self.prompter, connection, request).await {
            Ok(dialog) => Some(dialog),
            Err(error) => {
                tracing::warn!("cannot show the keychain prompt: {error}");
                None
            }
        }
    }

    async fn ask_unlock(self: &Arc<Self>, name: &str, app: Option<String>) -> bool {
        let (label, exists) = {
            let keychain = self.keychain.lock().await;
            match keychain.collection(name) {
                Some(collection) if !collection.locked() => return true,
                Some(collection) => (collection.label.clone(), keychain.exists(name)),
                None => return false,
            }
        };
        let attempt = (name == LOGIN && exists)
            .then(|| guard(&self.login_attempt).clone())
            .flatten();
        let kind = match (&attempt, exists) {
            (Some(_), _) => Kind::UpdateLogin,
            (None, true) => Kind::Unlock,
            (None, false) => Kind::Create,
        };
        let request = Request {
            kind,
            app,
            keychain: label,
            items: Vec::new(),
        };
        let Some(mut dialog) = self.dialog(&request).await else {
            return false;
        };
        loop {
            let password = match dialog.reply().await {
                Reply::Cancel => return false,
                Reply::CreateNew => {
                    let Some(attempt) = attempt.clone() else {
                        return false;
                    };
                    let created = {
                        let mut keychain = self.keychain.lock().await;
                        match keychain.set_aside(LOGIN) {
                            Ok(()) => keychain.unlock(LOGIN, attempt).await,
                            Err(error) => Err(error),
                        }
                    };
                    if let Err(error) = created {
                        tracing::warn!("cannot make a new login keychain: {error}");
                        return false;
                    }
                    break;
                }
                Reply::Allow(password) | Reply::Always(password) => password,
            };
            let unlocked = self.keychain.lock().await.unlock(name, password).await;
            match unlocked {
                Ok(()) => {
                    if let Some(login) = attempt.clone() {
                        let changed = self
                            .keychain
                            .lock()
                            .await
                            .change_password(LOGIN, login)
                            .await;
                        if let Err(error) = changed {
                            tracing::warn!("cannot update the login keychain password: {error}");
                        }
                    }
                    break;
                }
                Err(store::Error::WrongPassword) => {
                    if dialog.wrong().await.is_err() {
                        return false;
                    }
                }
                Err(error) => {
                    tracing::warn!("cannot unlock the {name} keychain: {error}");
                    return false;
                }
            }
        }
        dialog.finish().await;
        if name == LOGIN {
            *guard(&self.login_attempt) = None;
        }
        self.sync().await;
        self.locked_changed(name).await;
        true
    }

    async fn ask_allow(&self, caller: &Caller, items: Vec<(String, u64)>) -> bool {
        let Some((name, _)) = items.first().cloned() else {
            return true;
        };
        let (label, labels) = {
            let keychain = self.keychain.lock().await;
            let label = keychain
                .collection(&name)
                .map(|collection| collection.label.clone())
                .unwrap_or_default();
            let labels = items
                .iter()
                .filter_map(|(collection, id)| keychain.find(collection, *id).ok())
                .map(|item| item.label().to_owned())
                .collect();
            (label, labels)
        };
        let request = Request {
            kind: Kind::Allow,
            app: caller.identity.clone(),
            keychain: label,
            items: labels,
        };
        let Some(mut dialog) = self.dialog(&request).await else {
            return false;
        };
        loop {
            let (password, always) = match dialog.reply().await {
                Reply::Cancel | Reply::CreateNew => return false,
                Reply::Allow(password) => (password, false),
                Reply::Always(password) => (password, caller.identity.is_some()),
            };
            let verified = self.keychain.lock().await.verify(&name, &password).await;
            match verified {
                Ok(()) => {
                    if always && let Some(identity) = &caller.identity {
                        let mut keychain = self.keychain.lock().await;
                        for (collection, id) in &items {
                            let added = keychain
                                .update_item(collection, *id, |item| {
                                    let mut attributes = item.attributes().clone();
                                    let mut list = allowed_list(item)
                                        .into_iter()
                                        .map(str::to_owned)
                                        .collect::<Vec<_>>();
                                    if !list.contains(identity) {
                                        list.push(identity.clone());
                                    }
                                    attributes.insert(ALLOWED.to_owned(), list.join("\n"));
                                    item.set_attributes(&attributes);
                                })
                                .await;
                            if let Err(error) = added {
                                tracing::warn!("cannot remember the app for an item: {error}");
                            }
                        }
                    }
                    self.grant(
                        &caller.sender,
                        items
                            .iter()
                            .map(|(collection, id)| item_path(collection, *id)),
                    );
                    dialog.finish().await;
                    return true;
                }
                Err(store::Error::WrongPassword) => {
                    if dialog.wrong().await.is_err() {
                        return false;
                    }
                }
                Err(error) => {
                    tracing::warn!("cannot check the keychain password: {error}");
                    return false;
                }
            }
        }
    }

    async fn ask_create(self: &Arc<Self>, caller: &Caller, label: &str) -> Option<String> {
        let base = label
            .chars()
            .map(|character| match character {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => character,
                _ => '_',
            })
            .collect::<String>();
        let base = if base.is_empty() {
            "keychain".to_owned()
        } else {
            base
        };
        let name = {
            let keychain = self.keychain.lock().await;
            (1..)
                .map(|number| match number {
                    1 => base.clone(),
                    number => format!("{base}_{number}"),
                })
                .find(|name| keychain.collection(name).is_none() && !keychain.exists(name))?
        };
        let request = Request {
            kind: Kind::Create,
            app: caller.identity.clone(),
            keychain: label.to_owned(),
            items: Vec::new(),
        };
        let mut dialog = self.dialog(&request).await?;
        let password = match dialog.reply().await {
            Reply::Allow(password) | Reply::Always(password) => password,
            Reply::Cancel | Reply::CreateNew => return None,
        };
        let created = {
            let mut keychain = self.keychain.lock().await;
            let created = keychain.create_collection(&name, password).await;
            if created.is_ok() {
                let _ = keychain.rename(&name, label);
            }
            created
        };
        dialog.finish().await;
        if let Err(error) = created {
            tracing::warn!("cannot make the keychain: {error}");
            return None;
        }
        if let Err(error) = self.register_collection(&name).await {
            tracing::warn!("cannot publish the keychain: {error}");
        }
        Some(name)
    }

    async fn unlock_for_search(self: &Arc<Self>, caller: &Caller) {
        let _searching = self.searching.lock().await;
        let default = {
            let keychain = self.keychain.lock().await;
            let default = keychain.default_collection().to_owned();
            let waiting = keychain
                .collection(&default)
                .is_some_and(store::Collection::locked)
                && keychain.exists(&default);
            if !waiting {
                return;
            }
            default
        };
        if guard(&self.peers)
            .get(&caller.sender)
            .is_some_and(|peer| peer.declined)
        {
            return;
        }
        if !self.ask_unlock(&default, caller.identity.clone()).await {
            guard(&self.peers)
                .entry(caller.sender.clone())
                .or_default()
                .declined = true;
        }
    }

    async fn search(
        self: &Arc<Self>,
        caller: &Caller,
        only: Option<&str>,
        attributes: &HashMap<String, String>,
    ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
        let keychain = self.keychain.lock().await;
        let mut unlocked = Vec::new();
        let mut locked = Vec::new();
        for collection in keychain.collections() {
            if only.is_some_and(|only| only != collection.name) {
                continue;
            }
            for entry in collection.entries() {
                if !store::matches(entry.item.attributes(), attributes) {
                    continue;
                }
                let path = item_path(&collection.name, entry.id);
                if self.may_use(caller, &path, entry.item) {
                    unlocked.push(path);
                } else {
                    locked.push(path);
                }
            }
        }
        (unlocked, locked)
    }

    async fn new_prompt(self: &Arc<Self>, owner: &str, job: Job) -> Result<OwnedObjectPath> {
        let path = object_path(format!("{SERVICE_PATH}/prompt/p{}", self.next()));
        guard(&self.prompts).insert(
            path.clone(),
            Pending {
                owner: owner.to_owned(),
                refusal: job.refusal(),
                job: Some(job),
                task: None,
            },
        );
        self.connection()?
            .object_server()
            .at(
                &path,
                PromptObject {
                    daemon: self.clone(),
                    path: path.clone(),
                },
            )
            .await?;
        Ok(path)
    }

    async fn run_job(self: &Arc<Self>, owner: &str, job: Job) -> Option<OwnedValue> {
        let caller = Caller {
            sender: owner.to_owned(),
            identity: guard(&self.peers)
                .get(owner)
                .and_then(|peer| peer.identity.clone())
                .flatten(),
        };
        match job {
            Job::Unlock(objects) => {
                let targets = {
                    let keychain = self.keychain.lock().await;
                    objects
                        .iter()
                        .filter_map(|path| target(&keychain, path.as_str()))
                        .collect::<Vec<_>>()
                };
                let mut collections = Vec::new();
                for target in &targets {
                    let (Target::Collection(name) | Target::Item(name, _)) = target;
                    if !collections.contains(name) {
                        collections.push(name.clone());
                    }
                }
                for name in &collections {
                    if !self.ask_unlock(name, caller.identity.clone()).await {
                        return None;
                    }
                }
                let refused = {
                    let keychain = self.keychain.lock().await;
                    targets
                        .iter()
                        .filter_map(|target| match target {
                            Target::Item(name, id) => {
                                let item = keychain.find(name, *id).ok()?;
                                (!self.may_use(&caller, &item_path(name, *id), item))
                                    .then(|| (name.clone(), *id))
                            }
                            Target::Collection(_) => None,
                        })
                        .collect::<Vec<_>>()
                };
                if !refused.is_empty() && !self.ask_allow(&caller, refused).await {
                    return None;
                }
                Value::from(objects).try_into_owned().ok()
            }
            Job::Create { label, alias } => {
                let name = self.ask_create(&caller, &label).await?;
                if alias == store::DEFAULT {
                    let set = self.keychain.lock().await.set_default(&name);
                    if let Err(error) = set {
                        tracing::warn!("cannot make the new keychain the default: {error}");
                    } else if let Err(error) = self.register_aliases().await {
                        tracing::warn!("cannot publish the default keychain: {error}");
                    }
                }
                self.emit_collection_created(&name).await;
                Value::from(collection_path(&name)).try_into_owned().ok()
            }
        }
    }

    async fn emit_collection_created(&self, name: &str) {
        if let Ok(connection) = self.connection()
            && let Ok(emitter) = SignalEmitter::new(connection, SERVICE_PATH)
        {
            let _ =
                ServiceObject::collection_created(&emitter, collection_path(name).as_ref()).await;
        }
    }

    async fn complete(self: &Arc<Self>, path: &OwnedObjectPath, result: Option<OwnedValue>) {
        let pending = guard(&self.prompts).remove(path);
        let Some(pending) = pending else {
            return;
        };
        if let Some(task) = pending.task {
            task.detach();
        }
        let Ok(connection) = self.connection() else {
            return;
        };
        if let Ok(emitter) = SignalEmitter::new(connection, path.as_ref()) {
            let dismissed = result.is_none();
            let result = result.unwrap_or(pending.refusal);
            let _ = PromptObject::completed(&emitter, dismissed, result).await;
        }
        let _ = connection
            .object_server()
            .remove::<PromptObject, _>(path)
            .await;
    }

    fn session_for(&self, path: &OwnedObjectPath, sender: &str) -> Result<()> {
        match guard(&self.sessions).get(path) {
            Some(session) if session.owner == sender => Ok(()),
            _ => Err(Error::NoSession("no such session".to_owned())),
        }
    }

    fn encode(&self, path: &OwnedObjectPath, sender: &str, secret: &Secret) -> Result<WireSecret> {
        let sessions = guard(&self.sessions);
        let session = sessions
            .get(path)
            .filter(|session| session.owner == sender)
            .ok_or_else(|| Error::NoSession("no such session".to_owned()))?;
        let (parameters, value) = session.encode(secret.as_bytes())?;
        Ok((
            path.clone(),
            parameters,
            value,
            store::content_type(secret).to_owned(),
        ))
    }

    fn decode(&self, secret: &WireSecret, sender: &str) -> Result<Secret> {
        let (path, parameters, value, content_type) = secret;
        let sessions = guard(&self.sessions);
        let session = sessions
            .get(path)
            .filter(|session| session.owner == sender)
            .ok_or_else(|| Error::NoSession("no such session".to_owned()))?;
        let plain = session.decode(parameters, value)?;
        Ok(store::secret_of(content_type, &plain))
    }
}

pub struct ServiceObject {
    daemon: Arc<Daemon>,
}

#[interface(name = "org.freedesktop.Secret.Service")]
impl ServiceObject {
    async fn open_session(
        &self,
        algorithm: &str,
        input: Value<'_>,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(OwnedValue, OwnedObjectPath)> {
        let sender = header
            .sender()
            .map(|sender| sender.as_str().to_owned())
            .unwrap_or_default();
        let input = match input {
            Value::Value(inner) => *inner,
            other => other,
        };
        let (session, output) = Session::open(&sender, algorithm, &input)?;
        let path = object_path(format!("{SERVICE_PATH}/session/s{}", self.daemon.next()));
        guard(&self.daemon.sessions).insert(path.clone(), session);
        self.daemon
            .connection()?
            .object_server()
            .at(
                &path,
                SessionObject {
                    daemon: self.daemon.clone(),
                    path: path.clone(),
                },
            )
            .await?;
        Ok((output, path))
    }

    async fn create_collection(
        &self,
        properties: HashMap<String, OwnedValue>,
        alias: &str,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath)> {
        let existing = {
            let keychain = self.daemon.keychain.lock().await;
            match alias {
                store::DEFAULT => Some(keychain.default_collection().to_owned()),
                SESSION => Some(SESSION.to_owned()),
                _ => None,
            }
        };
        if let Some(name) = existing {
            return Ok((collection_path(&name), no_prompt()));
        }
        let label = properties
            .get(COLLECTION_LABEL)
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
            .unwrap_or_else(|| "Keychain".to_owned());
        let caller = self.daemon.caller(&header).await;
        let prompt = self
            .daemon
            .new_prompt(
                &caller.sender,
                Job::Create {
                    label,
                    alias: alias.to_owned(),
                },
            )
            .await?;
        Ok((no_prompt(), prompt))
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)> {
        let caller = self.daemon.caller(&header).await;
        self.daemon.unlock_for_search(&caller).await;
        Ok(self.daemon.search(&caller, None, &attributes).await)
    }

    async fn unlock(
        &self,
        objects: Vec<OwnedObjectPath>,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(Vec<OwnedObjectPath>, OwnedObjectPath)> {
        let caller = self.daemon.caller(&header).await;
        let mut unlocked = Vec::new();
        let mut waiting = Vec::new();
        {
            let keychain = self.daemon.keychain.lock().await;
            for path in objects {
                let open = match target(&keychain, path.as_str()) {
                    Some(Target::Collection(name)) => keychain
                        .collection(&name)
                        .is_some_and(|collection| !collection.locked()),
                    Some(Target::Item(name, id)) => keychain
                        .find(&name, id)
                        .is_ok_and(|item| self.daemon.may_use(&caller, &path, item)),
                    None => continue,
                };
                if open {
                    unlocked.push(path);
                } else {
                    waiting.push(path);
                }
            }
        }
        if waiting.is_empty() {
            return Ok((unlocked, no_prompt()));
        }
        let prompt = self
            .daemon
            .new_prompt(&caller.sender, Job::Unlock(waiting))
            .await?;
        Ok((unlocked, prompt))
    }

    async fn lock(
        &self,
        objects: Vec<OwnedObjectPath>,
    ) -> Result<(Vec<OwnedObjectPath>, OwnedObjectPath)> {
        let mut locked = Vec::new();
        let mut names = Vec::new();
        {
            let mut keychain = self.daemon.keychain.lock().await;
            for path in objects {
                let (Some(Target::Collection(name)) | Some(Target::Item(name, _))) =
                    target(&keychain, path.as_str())
                else {
                    continue;
                };
                if name != SESSION && keychain.lock(&name).is_ok() {
                    locked.push(path);
                    names.push(name);
                }
            }
        }
        self.daemon.revoke_grants();
        self.daemon.sync().await;
        for name in names {
            self.daemon.locked_changed(&name).await;
        }
        Ok((locked, no_prompt()))
    }

    async fn get_secrets(
        &self,
        items: Vec<OwnedObjectPath>,
        session: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<HashMap<OwnedObjectPath, WireSecret>> {
        let caller = self.daemon.caller(&header).await;
        self.daemon.session_for(&session, &caller.sender)?;
        let keychain = self.daemon.keychain.lock().await;
        let mut secrets = HashMap::new();
        for path in items {
            let Some(Target::Item(name, id)) = target(&keychain, path.as_str()) else {
                continue;
            };
            let Ok(item) = keychain.find(&name, id) else {
                continue;
            };
            if !self.daemon.may_use(&caller, &path, item) {
                continue;
            }
            let secret = self
                .daemon
                .encode(&session, &caller.sender, &item.secret())?;
            secrets.insert(path, secret);
        }
        Ok(secrets)
    }

    async fn read_alias(&self, name: &str) -> OwnedObjectPath {
        let keychain = self.daemon.keychain.lock().await;
        match name {
            store::DEFAULT => collection_path(keychain.default_collection()),
            SESSION => collection_path(SESSION),
            _ => no_prompt(),
        }
    }

    async fn set_alias(&self, name: &str, collection: OwnedObjectPath) -> Result<()> {
        if name != store::DEFAULT {
            return Err(fdo(zbus::fdo::Error::NotSupported(
                "only the default alias can be set".to_owned(),
            )));
        }
        {
            let mut keychain = self.daemon.keychain.lock().await;
            let Some(Target::Collection(target)) = target(&keychain, collection.as_str()) else {
                return Err(Error::NoSuchObject("no such keychain".to_owned()));
            };
            keychain.set_default(&target)?;
        }
        self.daemon.register_aliases().await?;
        Ok(())
    }

    #[zbus(property)]
    async fn collections(&self) -> Vec<OwnedObjectPath> {
        self.daemon
            .keychain
            .lock()
            .await
            .collections()
            .iter()
            .map(|collection| collection_path(&collection.name))
            .collect()
    }

    #[zbus(signal)]
    async fn collection_created(
        emitter: &SignalEmitter<'_>,
        collection: ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn collection_deleted(
        emitter: &SignalEmitter<'_>,
        collection: ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn collection_changed(
        emitter: &SignalEmitter<'_>,
        collection: ObjectPath<'_>,
    ) -> zbus::Result<()>;
}

pub struct CollectionObject {
    daemon: Arc<Daemon>,
    name: String,
}

impl CollectionObject {
    async fn emit(&self, created: bool, id: u64) {
        let Ok(connection) = self.daemon.connection() else {
            return;
        };
        let Ok(emitter) = SignalEmitter::new(connection, collection_path(&self.name)) else {
            return;
        };
        let path = item_path(&self.name, id);
        let _ = if created {
            Self::item_created(&emitter, path.as_ref()).await
        } else {
            Self::item_changed(&emitter, path.as_ref()).await
        };
    }
}

#[interface(name = "org.freedesktop.Secret.Collection")]
impl CollectionObject {
    async fn delete(&self) -> Result<OwnedObjectPath> {
        self.daemon
            .keychain
            .lock()
            .await
            .delete_collection(&self.name)?;
        self.daemon.sync().await;
        let connection = self.daemon.connection()?;
        let path = collection_path(&self.name);
        connection
            .object_server()
            .remove::<CollectionObject, _>(&path)
            .await?;
        if let Err(error) = self.daemon.register_aliases().await {
            tracing::warn!("cannot publish the default keychain: {error}");
        }
        let emitter = SignalEmitter::new(connection, SERVICE_PATH)?;
        ServiceObject::collection_deleted(&emitter, path.as_ref()).await?;
        Ok(no_prompt())
    }

    async fn search_items(
        &self,
        attributes: HashMap<String, String>,
        #[zbus(header)] header: Header<'_>,
    ) -> Vec<OwnedObjectPath> {
        let caller = self.daemon.caller(&header).await;
        let (mut unlocked, locked) = self
            .daemon
            .search(&caller, Some(&self.name), &attributes)
            .await;
        unlocked.extend(locked);
        unlocked
    }

    async fn create_item(
        &self,
        properties: HashMap<String, OwnedValue>,
        secret: WireSecret,
        replace: bool,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<(OwnedObjectPath, OwnedObjectPath)> {
        let caller = self.daemon.caller(&header).await;
        let secret = self.daemon.decode(&secret, &caller.sender)?;
        let label = properties
            .get(ITEM_LABEL)
            .and_then(|value| String::try_from(value.try_clone().ok()?).ok())
            .unwrap_or_default();
        let mut attributes = properties
            .get(ITEM_ATTRIBUTES)
            .and_then(|value| HashMap::<String, String>::try_from(value.try_clone().ok()?).ok())
            .map(client_attributes)
            .unwrap_or_default();
        let replaced = {
            let mut keychain = self.daemon.keychain.lock().await;
            let collection = keychain
                .collection(&self.name)
                .ok_or_else(|| Error::NoSuchObject("no such keychain".to_owned()))?;
            if collection.locked() {
                return Err(Error::IsLocked("the keychain is locked".to_owned()));
            }
            let existing = replace
                .then(|| {
                    collection
                        .entries()
                        .find(|entry| store::same_attributes(entry.item.attributes(), &attributes))
                        .map(|entry| {
                            (
                                entry.id,
                                entry.item.attributes().clone(),
                                self.daemon.may_use(
                                    &caller,
                                    &item_path(&self.name, entry.id),
                                    entry.item,
                                ),
                            )
                        })
                })
                .flatten();
            match existing {
                Some((_, _, false)) => {
                    return Err(Error::IsLocked(
                        "another app's item has these attributes".to_owned(),
                    ));
                }
                Some((id, stored, true)) => {
                    let attributes = keep_internal(&stored, attributes.clone());
                    keychain
                        .update_item(&self.name, id, |item| {
                            item.set_label(&label);
                            item.set_attributes(&attributes);
                            item.set_secret(secret.clone());
                        })
                        .await?;
                    Some(id)
                }
                None => None,
            }
        };
        if let Some(id) = replaced {
            self.emit(false, id).await;
            return Ok((item_path(&self.name, id), no_prompt()));
        }
        if let Some(identity) = &caller.identity {
            attributes.insert(CREATOR.to_owned(), identity.clone());
        }
        let id = self
            .daemon
            .keychain
            .lock()
            .await
            .create_item(&self.name, &label, &attributes, secret)
            .await?;
        self.daemon.sync().await;
        self.emit(true, id).await;
        Ok((item_path(&self.name, id), no_prompt()))
    }

    #[zbus(property)]
    async fn items(&self) -> Vec<OwnedObjectPath> {
        self.daemon
            .keychain
            .lock()
            .await
            .collection(&self.name)
            .map(|collection| {
                collection
                    .entries()
                    .map(|entry| item_path(&self.name, entry.id))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn label(&self) -> String {
        self.daemon
            .keychain
            .lock()
            .await
            .collection(&self.name)
            .map(|collection| collection.label.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn set_label(&mut self, label: String) -> zbus::fdo::Result<()> {
        self.daemon
            .keychain
            .lock()
            .await
            .rename(&self.name, &label)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    #[zbus(property)]
    async fn locked(&self) -> bool {
        self.daemon
            .keychain
            .lock()
            .await
            .collection(&self.name)
            .is_none_or(store::Collection::locked)
    }

    #[zbus(property)]
    async fn created(&self) -> u64 {
        self.daemon
            .keychain
            .lock()
            .await
            .collection(&self.name)
            .map(|collection| collection.created)
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn modified(&self) -> u64 {
        self.daemon
            .keychain
            .lock()
            .await
            .collection(&self.name)
            .map(|collection| collection.modified)
            .unwrap_or_default()
    }

    #[zbus(signal)]
    async fn item_created(emitter: &SignalEmitter<'_>, item: ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_deleted(emitter: &SignalEmitter<'_>, item: ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_changed(emitter: &SignalEmitter<'_>, item: ObjectPath<'_>) -> zbus::Result<()>;
}

pub struct ItemObject {
    daemon: Arc<Daemon>,
    collection: String,
    id: u64,
}

impl ItemObject {
    fn path(&self) -> OwnedObjectPath {
        item_path(&self.collection, self.id)
    }

    async fn usable(&self, header: &Header<'_>) -> Result<Caller> {
        let caller = self.daemon.caller(header).await;
        let keychain = self.daemon.keychain.lock().await;
        let item = keychain.find(&self.collection, self.id)?;
        if self.daemon.may_use(&caller, &self.path(), item) {
            Ok(caller)
        } else {
            Err(Error::IsLocked(
                "the item is locked for this app".to_owned(),
            ))
        }
    }

    async fn change(
        &self,
        header: Option<Header<'_>>,
        change: impl FnOnce(&mut UnlockedItem),
    ) -> zbus::fdo::Result<()> {
        if let Some(header) = &header {
            self.usable(header)
                .await
                .map_err(|_| zbus::fdo::Error::AccessDenied("the item is locked".to_owned()))?;
        }
        self.daemon
            .keychain
            .lock()
            .await
            .update_item(&self.collection, self.id, change)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        if let Ok(connection) = self.daemon.connection()
            && let Ok(emitter) = SignalEmitter::new(connection, collection_path(&self.collection))
        {
            let _ = CollectionObject::item_changed(&emitter, self.path().as_ref()).await;
        }
        Ok(())
    }

    async fn read<T>(&self, read: impl FnOnce(&UnlockedItem) -> T) -> Option<T> {
        self.daemon
            .keychain
            .lock()
            .await
            .find(&self.collection, self.id)
            .ok()
            .map(read)
    }
}

#[interface(name = "org.freedesktop.Secret.Item")]
impl ItemObject {
    async fn delete(&self, #[zbus(header)] header: Header<'_>) -> Result<OwnedObjectPath> {
        self.usable(&header).await?;
        self.daemon
            .keychain
            .lock()
            .await
            .delete_item(&self.collection, self.id)
            .await?;
        let connection = self.daemon.connection()?;
        let emitter = SignalEmitter::new(connection, collection_path(&self.collection))?;
        CollectionObject::item_deleted(&emitter, self.path().as_ref()).await?;
        let daemon = self.daemon.clone();
        connection
            .executor()
            .spawn(async move { daemon.sync().await }, "bande-secrets sync")
            .detach();
        Ok(no_prompt())
    }

    async fn get_secret(
        &self,
        session: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<WireSecret> {
        let caller = self.usable(&header).await?;
        let secret = self
            .read(UnlockedItem::secret)
            .await
            .ok_or_else(|| Error::NoSuchObject("no such item".to_owned()))?;
        self.daemon.encode(&session, &caller.sender, &secret)
    }

    async fn set_secret(
        &self,
        secret: WireSecret,
        #[zbus(header)] header: Header<'_>,
    ) -> Result<()> {
        let caller = self.usable(&header).await?;
        let secret = self.daemon.decode(&secret, &caller.sender)?;
        self.change(None, |item| item.set_secret(secret))
            .await
            .map_err(|error| failed(error.to_string()))
    }

    #[zbus(property)]
    async fn locked(&self, #[zbus(header)] header: Option<Header<'_>>) -> bool {
        let Some(header) = header else {
            return false;
        };
        self.usable(&header).await.is_err()
    }

    #[zbus(property)]
    async fn attributes(&self) -> HashMap<String, String> {
        self.read(|item| item.attributes().clone())
            .await
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn set_attributes(
        &mut self,
        attributes: HashMap<String, String>,
        #[zbus(header)] header: Option<Header<'_>>,
    ) -> zbus::fdo::Result<()> {
        let attributes = client_attributes(attributes);
        self.change(header, |item| {
            let attributes = keep_internal(item.attributes(), attributes);
            item.set_attributes(&attributes);
        })
        .await
    }

    #[zbus(property)]
    async fn label(&self) -> String {
        self.read(|item| item.label().to_owned())
            .await
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn set_label(
        &mut self,
        label: String,
        #[zbus(header)] header: Option<Header<'_>>,
    ) -> zbus::fdo::Result<()> {
        self.change(header, |item| item.set_label(label)).await
    }

    #[zbus(property)]
    async fn created(&self) -> u64 {
        self.read(|item| item.created().as_secs())
            .await
            .unwrap_or_default()
    }

    #[zbus(property)]
    async fn modified(&self) -> u64 {
        self.read(|item| item.modified().as_secs())
            .await
            .unwrap_or_default()
    }
}

pub struct SessionObject {
    daemon: Arc<Daemon>,
    path: OwnedObjectPath,
}

#[interface(name = "org.freedesktop.Secret.Session")]
impl SessionObject {
    async fn close(&self, #[zbus(header)] header: Header<'_>) -> Result<()> {
        let sender = header
            .sender()
            .map(|sender| sender.as_str().to_owned())
            .unwrap_or_default();
        self.daemon.session_for(&self.path, &sender)?;
        guard(&self.daemon.sessions).remove(&self.path);
        let path = self.path.clone();
        let connection = self.daemon.connection()?.clone();
        connection
            .executor()
            .spawn(
                {
                    let connection = connection.clone();
                    async move {
                        let _ = connection
                            .object_server()
                            .remove::<SessionObject, _>(&path)
                            .await;
                    }
                },
                "bande-secrets session",
            )
            .detach();
        Ok(())
    }
}

pub struct PromptObject {
    daemon: Arc<Daemon>,
    path: OwnedObjectPath,
}

#[interface(name = "org.freedesktop.Secret.Prompt")]
impl PromptObject {
    async fn prompt(&self, _window_id: &str, #[zbus(header)] header: Header<'_>) -> Result<()> {
        let sender = header
            .sender()
            .map(|sender| sender.as_str().to_owned())
            .unwrap_or_default();
        let job = {
            let mut prompts = guard(&self.daemon.prompts);
            let pending = prompts
                .get_mut(&self.path)
                .filter(|pending| pending.owner == sender)
                .ok_or_else(|| Error::NoSuchObject("no such prompt".to_owned()))?;
            pending.job.take()
        };
        let Some(job) = job else {
            return Ok(());
        };
        let daemon = self.daemon.clone();
        let path = self.path.clone();
        let task = self.daemon.connection()?.executor().spawn(
            async move {
                let result = daemon.run_job(&sender, job).await;
                daemon.complete(&path, result).await;
            },
            "bande-secrets prompt",
        );
        if let Some(pending) = guard(&self.daemon.prompts).get_mut(&self.path) {
            pending.task = Some(task);
        } else {
            task.detach();
        }
        Ok(())
    }

    async fn dismiss(&self, #[zbus(header)] header: Header<'_>) -> Result<()> {
        let sender = header
            .sender()
            .map(|sender| sender.as_str().to_owned())
            .unwrap_or_default();
        let owned = guard(&self.daemon.prompts)
            .get(&self.path)
            .is_some_and(|pending| pending.owner == sender);
        if !owned {
            return Err(Error::NoSuchObject("no such prompt".to_owned()));
        }
        let task = guard(&self.daemon.prompts)
            .get_mut(&self.path)
            .and_then(|pending| pending.task.take());
        drop(task);
        let daemon = self.daemon.clone();
        let path = self.path.clone();
        self.daemon
            .connection()?
            .executor()
            .spawn(
                async move { daemon.complete(&path, None).await },
                "bande-secrets dismiss",
            )
            .detach();
        Ok(())
    }

    #[zbus(signal)]
    async fn completed(
        emitter: &SignalEmitter<'_>,
        dismissed: bool,
        result: OwnedValue,
    ) -> zbus::Result<()>;
}
