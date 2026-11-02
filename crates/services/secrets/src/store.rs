use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use oo7::file::{self, UnlockedItem, UnlockedKeyring};
use oo7::{ContentType, Secret};

pub const LOGIN: &str = "login";
pub const SESSION: &str = "session";
pub const DEFAULT: &str = "default";
pub const CREATOR: &str = "bande:creator";
pub const ALLOWED: &str = "bande:allowed";
const CONTENT_TYPE: &str = oo7::CONTENT_TYPE_ATTRIBUTE;
const EXTENSION: &str = "keyring";
const OWN_FOLDER: &str = "bande";
const KEYRINGS: &str = "keyrings/v1";

#[derive(Debug)]
pub enum Error {
    File(file::Error),
    Io(std::io::Error),
    WrongPassword,
    Locked,
    NoSuchCollection,
    NoSuchItem,
    Protected,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(error) => write!(f, "keyring file: {error}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::WrongPassword => f.write_str("the password does not open the keychain"),
            Self::Locked => f.write_str("the keychain is locked"),
            Self::NoSuchCollection => f.write_str("no such keychain"),
            Self::NoSuchItem => f.write_str("no such item"),
            Self::Protected => f.write_str("the login and session keychains cannot be deleted"),
        }
    }
}

impl std::error::Error for Error {}

impl From<file::Error> for Error {
    fn from(error: file::Error) -> Self {
        match error {
            file::Error::IncorrectSecret
            | file::Error::ChecksumMismatch
            | file::Error::MacError => Self::WrongPassword,
            other => Self::File(other),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

enum Slot {
    Item { id: u64, item: UnlockedItem },
    Unreadable,
}

struct Open {
    keyring: UnlockedKeyring,
    slots: Vec<Slot>,
}

pub struct Collection {
    pub name: String,
    pub label: String,
    pub created: u64,
    pub modified: u64,
    open: Option<Open>,
}

pub struct Entry<'a> {
    pub id: u64,
    pub item: &'a UnlockedItem,
}

impl Collection {
    pub fn locked(&self) -> bool {
        self.open.is_none()
    }

    pub fn entries(&self) -> impl Iterator<Item = Entry<'_>> {
        self.open.iter().flat_map(|open| {
            open.slots.iter().filter_map(|slot| match slot {
                Slot::Item { id, item } => Some(Entry { id: *id, item }),
                Slot::Unreadable => None,
            })
        })
    }

    pub fn item(&self, id: u64) -> Option<&UnlockedItem> {
        self.entries()
            .find(|entry| entry.id == id)
            .map(|entry| entry.item)
    }

    fn open(&self) -> Result<&Open> {
        self.open.as_ref().ok_or(Error::Locked)
    }

    fn index(&self, id: u64) -> Result<usize> {
        self.open()?
            .slots
            .iter()
            .position(|slot| matches!(slot, Slot::Item { id: found, .. } if *found == id))
            .ok_or(Error::NoSuchItem)
    }
}

pub fn label_of(name: &str) -> String {
    match name {
        LOGIN => "Login".to_owned(),
        SESSION => "Session".to_owned(),
        other => other.replace('_', " "),
    }
}

pub fn content_type(secret: &Secret) -> &'static str {
    secret.content_type().as_str()
}

pub fn secret_of(content_type: &str, value: &[u8]) -> Secret {
    if content_type.starts_with("text/plain") {
        Secret::with_content_type(ContentType::Text, value)
    } else {
        Secret::with_content_type(ContentType::Blob, value)
    }
}

pub fn same_attributes(stored: &HashMap<String, String>, wanted: &HashMap<String, String>) -> bool {
    let visible = |map: &HashMap<String, String>| {
        map.iter()
            .filter(|(key, _)| !internal(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<HashMap<_, _>>()
    };
    visible(stored) == visible(wanted)
}

pub fn internal(key: &str) -> bool {
    key == CONTENT_TYPE || key.starts_with("bande:")
}

pub fn matches(stored: &HashMap<String, String>, wanted: &HashMap<String, String>) -> bool {
    wanted
        .iter()
        .all(|(key, value)| stored.get(key) == Some(value))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn modified_seconds(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
}

fn keyring_names(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut names = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == EXTENSION)
        })
        .filter_map(|path| path.file_stem()?.to_str().map(str::to_owned))
        .collect::<Vec<_>>();
    names.sort();
    names
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub struct Keychain {
    data_home: PathBuf,
    collections: Vec<Collection>,
    default: String,
    next_id: u64,
}

impl Keychain {
    pub async fn load(data_home: PathBuf) -> Self {
        let data_home = data_home.join(OWN_FOLDER);
        let directory = data_home.join(KEYRINGS);
        let mut names = keyring_names(&directory);
        if !names.iter().any(|name| name == LOGIN) {
            names.insert(0, LOGIN.to_owned());
        }
        names.retain(|name| valid_name(name) && name != SESSION);
        let default = std::fs::read_to_string(directory.join(DEFAULT))
            .ok()
            .map(|text| text.trim().to_owned())
            .filter(|name| names.contains(name))
            .unwrap_or_else(|| LOGIN.to_owned());
        let mut keychain = Self {
            data_home,
            collections: Vec::new(),
            default,
            next_id: 1,
        };
        for name in names {
            let path = keychain.file(&name);
            let created = modified_seconds(&path).unwrap_or_else(now);
            keychain.collections.push(Collection {
                label: label_of(&name),
                name,
                created,
                modified: created,
                open: None,
            });
        }
        let session = match UnlockedKeyring::temporary_unencrypted().await {
            Ok(keyring) => Some(Open {
                keyring,
                slots: Vec::new(),
            }),
            Err(error) => {
                tracing::warn!("cannot make the session keychain: {error}");
                None
            }
        };
        keychain.collections.push(Collection {
            name: SESSION.to_owned(),
            label: label_of(SESSION),
            created: now(),
            modified: now(),
            open: session,
        });
        keychain
    }

    fn directory(&self) -> PathBuf {
        self.data_home.join(KEYRINGS)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.directory().join(format!("{name}.{EXTENSION}"))
    }

    pub fn collections(&self) -> &[Collection] {
        &self.collections
    }

    pub fn collection(&self, name: &str) -> Option<&Collection> {
        self.collections
            .iter()
            .find(|collection| collection.name == name)
    }

    fn collection_mut(&mut self, name: &str) -> Result<&mut Collection> {
        self.collections
            .iter_mut()
            .find(|collection| collection.name == name)
            .ok_or(Error::NoSuchCollection)
    }

    pub fn default_collection(&self) -> &str {
        &self.default
    }

    pub fn set_default(&mut self, name: &str) -> Result<()> {
        if self.collection(name).is_none() || name == SESSION {
            return Err(Error::NoSuchCollection);
        }
        let directory = self.directory();
        std::fs::create_dir_all(&directory)?;
        let temporary = directory.join(".default.new");
        std::fs::write(&temporary, name)?;
        std::fs::rename(&temporary, directory.join(DEFAULT))?;
        name.clone_into(&mut self.default);
        Ok(())
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn exists(&self, name: &str) -> bool {
        self.file(name).exists()
    }

    pub async fn unlock(&mut self, name: &str, password: Secret) -> Result<()> {
        if !self.collection_mut(name)?.locked() {
            return self.verify(name, &password).await;
        }
        let keyring = UnlockedKeyring::open_at(&self.data_home, name, Some(password)).await?;
        let mut slots = Vec::new();
        for item in keyring.all_items().await? {
            slots.push(match item {
                Ok(item) => Slot::Item {
                    id: self.fresh_id(),
                    item,
                },
                Err(_) => Slot::Unreadable,
            });
        }
        let modified = modified_seconds(&self.file(name));
        let collection = self.collection_mut(name)?;
        collection.modified = modified.unwrap_or(collection.modified);
        collection.open = Some(Open { keyring, slots });
        Ok(())
    }

    pub async fn verify(&self, name: &str, password: &Secret) -> Result<()> {
        let collection = self.collection(name).ok_or(Error::NoSuchCollection)?;
        match collection.open()?.keyring.validate_secret(password).await? {
            true => Ok(()),
            false => Err(Error::WrongPassword),
        }
    }

    pub fn lock(&mut self, name: &str) -> Result<()> {
        if name == SESSION {
            return Ok(());
        }
        self.collection_mut(name)?.open = None;
        Ok(())
    }

    pub async fn create_collection(&mut self, name: &str, password: Secret) -> Result<()> {
        if self.collection(name).is_none() {
            self.collections.push(Collection {
                name: name.to_owned(),
                label: label_of(name),
                created: now(),
                modified: now(),
                open: None,
            });
        }
        self.unlock(name, password).await?;
        let open = self.collection_mut(name)?.open()?;
        open.keyring.write().await?;
        Ok(())
    }

    pub fn delete_collection(&mut self, name: &str) -> Result<()> {
        if name == SESSION || name == LOGIN {
            return Err(Error::Protected);
        }
        let file = self.file(name);
        if file.exists() {
            std::fs::remove_file(file)?;
        }
        self.collections
            .retain(|collection| collection.name != name);
        if self.default == name {
            self.default = LOGIN.to_owned();
        }
        Ok(())
    }

    pub async fn change_password(&mut self, name: &str, password: Secret) -> Result<()> {
        let collection = self.collection(name).ok_or(Error::NoSuchCollection)?;
        collection.open()?.keyring.change_secret(password).await?;
        self.reload(name).await
    }

    async fn reload(&mut self, name: &str) -> Result<()> {
        let keyring_items = self
            .collection(name)
            .ok_or(Error::NoSuchCollection)?
            .open()?
            .keyring
            .all_items()
            .await?;
        let previous = self
            .collection(name)
            .map(|collection| {
                collection
                    .entries()
                    .map(|entry| entry.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut ids = previous.into_iter();
        let mut slots = Vec::new();
        for item in keyring_items {
            slots.push(match item {
                Ok(item) => {
                    let id = match ids.next() {
                        Some(id) => id,
                        None => self.fresh_id(),
                    };
                    Slot::Item { id, item }
                }
                Err(_) => Slot::Unreadable,
            });
        }
        let collection = self.collection_mut(name)?;
        if let Some(open) = &mut collection.open {
            open.slots = slots;
        }
        collection.modified = now();
        Ok(())
    }

    pub async fn create_item(
        &mut self,
        name: &str,
        label: &str,
        attributes: &HashMap<String, String>,
        secret: Secret,
    ) -> Result<u64> {
        let id = self.fresh_id();
        let collection = self.collection_mut(name)?;
        let open = collection.open.as_mut().ok_or(Error::Locked)?;
        let item = open
            .keyring
            .create_item(label, attributes, secret, false)
            .await?;
        open.slots.push(Slot::Item { id, item });
        collection.modified = now();
        Ok(id)
    }

    pub async fn update_item(
        &mut self,
        name: &str,
        id: u64,
        change: impl FnOnce(&mut UnlockedItem),
    ) -> Result<()> {
        let collection = self.collection_mut(name)?;
        let index = collection.index(id)?;
        let open = collection.open.as_mut().ok_or(Error::Locked)?;
        let Some(Slot::Item { item, .. }) = open.slots.get_mut(index) else {
            return Err(Error::NoSuchItem);
        };
        let mut changed = item.clone();
        change(&mut changed);
        open.keyring.replace_item_index(index, &changed).await?;
        *item = changed;
        collection.modified = now();
        Ok(())
    }

    pub async fn delete_item(&mut self, name: &str, id: u64) -> Result<()> {
        let collection = self.collection_mut(name)?;
        let index = collection.index(id)?;
        let open = collection.open.as_mut().ok_or(Error::Locked)?;
        open.keyring.delete_item_index(index).await?;
        open.slots.remove(index);
        collection.modified = now();
        Ok(())
    }

    pub fn find(&self, name: &str, id: u64) -> Result<&UnlockedItem> {
        let collection = self.collection(name).ok_or(Error::NoSuchCollection)?;
        collection.open()?;
        collection.item(id).ok_or(Error::NoSuchItem)
    }

    pub fn rename(&mut self, name: &str, label: &str) -> Result<()> {
        label.clone_into(&mut self.collection_mut(name)?.label);
        Ok(())
    }

    pub fn set_aside(&mut self, name: &str) -> Result<()> {
        let file = self.file(name);
        if file.exists() {
            let aside = file.with_file_name(format!("{name}-{}.{EXTENSION}", now()));
            std::fs::rename(file, aside)?;
        }
        self.collection_mut(name)?.open = None;
        Ok(())
    }
}
