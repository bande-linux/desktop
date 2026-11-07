use std::io;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use bande_config::{Config, Search, SearchCategory, config_path};
use bande_inbox::{Inbox, Outbox};
use serde::{Deserialize, Serialize};
use zbus::blocking::Connection;
use zbus::blocking::fdo::PropertiesProxy;
use zbus::zvariant::Type;

use crate::index::MAX_HITS;
use crate::{FileHit, FileResults};

pub const BUS_NAME: &str = "org.bande.Search";
pub const OBJECT_PATH: &str = "/org/bande/Search";
const CONFIG_BUS_NAME: &str = "org.bande.Config";
const CONFIG_PATH: &str = "/org/bande/Config";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Found {
    pub domain: String,
    pub kind: String,
    pub target: String,
    pub title: String,
    pub detail: String,
    pub score: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Donation {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub target: String,
    pub body: String,
}

#[zbus::proxy(
    interface = "org.bande.Search",
    default_service = "org.bande.Search",
    default_path = "/org/bande/Search"
)]
pub trait Index {
    fn find(&self, text: &str, within: &str, limit: u32) -> zbus::Result<Vec<Found>>;
    fn donate(&self, domain: &str, items: Vec<Donation>, cursor: &str) -> zbus::Result<()>;
    fn cursor(&self, domain: &str) -> zbus::Result<String>;
    fn forget(&self, domain: &str, ids: Vec<String>) -> zbus::Result<()>;
    fn forget_all(&self, domain: &str) -> zbus::Result<()>;
}

struct Asked {
    generation: u64,
    text: String,
    within: String,
    limit: u32,
}

pub struct SearchService {
    asks: Sender<Asked>,
    results: Inbox<FileResults>,
}

fn ask(connection: &mut Option<Connection>, asked: &Asked) -> zbus::Result<Vec<Found>> {
    if connection.is_none() {
        *connection = Some(bande_bus::session()?);
    }
    let session = connection.as_ref().ok_or(zbus::Error::Unsupported)?;
    IndexProxyBlocking::new(session)?.find(&asked.text, &asked.within, asked.limit)
}

fn answering(asks: &Receiver<Asked>, results: &Outbox<FileResults>) {
    let mut connection = None;
    while let Ok(mut asked) = asks.recv() {
        while let Ok(newer) = asks.try_recv() {
            asked = newer;
        }
        let found = ask(&mut connection, &asked)
            .inspect_err(|error| {
                connection = None;
                tracing::warn!("cannot reach the search index: {error}");
            })
            .unwrap_or_default();
        let (files, items): (Vec<Found>, Vec<Found>) = found
            .into_iter()
            .partition(|found| found.kind == "file" || found.kind == "folder");
        let hits = files
            .into_iter()
            .map(|found| FileHit {
                folder: found.kind == "folder",
                path: PathBuf::from(found.target),
                score: found.score,
            })
            .collect();
        if !results.send(FileResults {
            generation: asked.generation,
            hits,
            items,
        }) {
            return;
        }
    }
}

impl SearchService {
    pub fn connect() -> io::Result<Self> {
        let (asks, received) = channel();
        let results = Inbox::new()?;
        let sender = results.outbox();
        thread::Builder::new()
            .name("search-client".to_owned())
            .spawn(move || answering(&received, &sender))?;
        Ok(Self { asks, results })
    }

    pub fn query(&self, generation: u64, text: &str) {
        self.query_within(generation, text, Path::new(""), MAX_HITS);
    }

    pub fn query_within(&self, generation: u64, text: &str, within: &Path, limit: usize) {
        let _ = self.asks.send(Asked {
            generation,
            text: text.to_owned(),
            within: within.to_string_lossy().into_owned(),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        });
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.results.fd()
    }

    pub fn take(&mut self) -> Option<FileResults> {
        self.results.drain().pop()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub excluded: Vec<PathBuf>,
    pub files: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            excluded: Vec::new(),
            files: true,
        }
    }
}

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config"
)]
trait ConfigService {
    #[zbus(property)]
    fn search_excluded(&self) -> zbus::Result<Vec<String>>;
    #[zbus(property)]
    fn search_shows(&self) -> zbus::Result<std::collections::HashMap<String, bool>>;
}

fn saved_search(home: &Path) -> Search {
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    config_path(config_home.as_deref(), Some(home))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::load(&source).ok())
        .map(|(config, _)| config.search)
        .unwrap_or_default()
}

fn policy(config: &ConfigServiceProxyBlocking<'_>, home: &Path) -> Policy {
    let (Ok(exclude), Ok(shows)) = (config.search_excluded(), config.search_shows()) else {
        let search = saved_search(home);
        return Policy {
            excluded: search.excluded(Some(home)),
            files: search.shows(SearchCategory::Documents) || search.shows(SearchCategory::Folders),
        };
    };
    let search = Search {
        exclude: exclude.into_iter().map(PathBuf::from).collect(),
        ..Search::default()
    };
    let shown = |category: SearchCategory| shows.get(category.keyword()).copied().unwrap_or(true);
    Policy {
        excluded: search.excluded(Some(home)),
        files: shown(SearchCategory::Documents) || shown(SearchCategory::Folders),
    }
}

pub fn follow_policy(home: PathBuf, mut changed: impl FnMut(Policy) -> bool + Send + 'static) {
    let work = move || {
        let mut follow = || -> zbus::Result<()> {
            let session = Connection::session()?;
            let config = ConfigServiceProxyBlocking::builder(&session)
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()?;
            let changes = PropertiesProxy::builder(&session)
                .destination(CONFIG_BUS_NAME)?
                .path(CONFIG_PATH)?
                .build()?
                .receive_properties_changed()?;
            let mut last = policy(&config, &home);
            if !changed(last.clone()) {
                return Ok(());
            }
            for _ in changes {
                let now = policy(&config, &home);
                if now != last {
                    last = now.clone();
                    if !changed(now) {
                        break;
                    }
                }
            }
            Ok(())
        };
        if let Err(error) = follow() {
            tracing::warn!("cannot follow the Search settings in org.bande.Config: {error}");
        }
    };
    if let Err(error) = thread::Builder::new()
        .name("search privacy".to_owned())
        .spawn(work)
    {
        tracing::warn!("cannot start following the Search settings: {error}");
    }
}
