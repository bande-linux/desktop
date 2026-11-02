use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::cache::Cache;
use crate::cookies::Jar;
use crate::hsts::Hsts;
use crate::{Clear, SiteData};
use std::collections::HashMap;

const COOKIES: &str = "cookies";
const HSTS: &str = "hsts";
const CACHE: &str = "cache";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Storage {
    Memory,
    Disk(PathBuf),
}

pub(crate) struct State {
    folder: Option<PathBuf>,
    pub(crate) cookies: Jar,
    pub(crate) hsts: Hsts,
    pub(crate) cache: Cache,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

pub(crate) fn replace(path: &Path, text: &str) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let staging = path.with_extension("new");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staging)?;
    file.write_all(text.as_bytes())?;
    std::fs::rename(&staging, path)
}

impl State {
    pub(crate) fn open(storage: &Storage) -> Self {
        let now = now();
        match storage {
            Storage::Memory => Self {
                folder: None,
                cookies: Jar::default(),
                hsts: Hsts::default(),
                cache: Cache::memory(),
            },
            Storage::Disk(folder) => {
                let read =
                    |name: &str| std::fs::read_to_string(folder.join(name)).unwrap_or_default();
                Self {
                    cookies: Jar::load(&read(COOKIES), now),
                    hsts: Hsts::load(&read(HSTS), now),
                    cache: Cache::on_disk(folder.join(CACHE)),
                    folder: Some(folder.clone()),
                }
            }
        }
    }

    pub(crate) fn clear(&mut self, clear: &Clear) {
        let sites = clear.sites.as_deref();
        if clear.cookies {
            self.cookies.clear(clear.since, sites);
        }
        if clear.cache {
            self.cache.clear(clear.since, sites);
        }
        self.persist();
    }

    pub(crate) fn site_data(&self) -> Vec<SiteData> {
        let mut counts = HashMap::new();
        self.cookies.sites(&mut counts);
        self.cache.sites(&mut counts);
        let mut sites: Vec<SiteData> = counts
            .into_iter()
            .filter(|(site, _)| !site.is_empty())
            .map(|(site, (cookies, cache_bytes))| SiteData {
                site,
                cookies,
                cache_bytes,
            })
            .collect();
        sites.sort_by(|a, b| a.site.cmp(&b.site));
        sites
    }

    pub(crate) fn persist(&mut self) {
        let Some(folder) = &self.folder else {
            return;
        };
        if self.cookies.take_changed() {
            let _ = replace(&folder.join(COOKIES), &self.cookies.save());
        }
        if self.hsts.take_changed() {
            let _ = replace(&folder.join(HSTS), &self.hsts.save());
        }
        self.cache.persist();
    }
}
