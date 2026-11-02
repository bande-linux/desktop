use std::collections::HashMap;
use std::path::PathBuf;

use hyper::HeaderMap;
use hyper::header::{
    AGE, CACHE_CONTROL, CONTENT_TYPE, DATE, ETAG, EXPIRES, LAST_MODIFIED, PRAGMA, VARY,
};
use url::Url;

use crate::date::parse_cookie_date;
use crate::{ContentType, Response};

const MEMORY_LIMIT: usize = 32 * 1024 * 1024;
const DISK_LIMIT: usize = 256 * 1024 * 1024;
const ENTRY_LIMIT: usize = 16 * 1024 * 1024;
const HEURISTIC_LIMIT: u64 = 24 * 60 * 60;
const INDEX: &str = "index";

#[derive(Clone)]
struct Entry {
    url: Url,
    partition: String,
    status: u16,
    content_type: String,
    stored: u64,
    lifetime: u64,
    etag: Option<String>,
    last_modified: Option<String>,
    size: usize,
    used: u64,
    body: Option<Vec<u8>>,
}

pub(crate) enum Lookup {
    Fresh(Response),
    Stale {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    Miss,
}

pub(crate) struct Cache {
    folder: Option<PathBuf>,
    entries: HashMap<u64, Entry>,
    total: usize,
    clock: u64,
    changed: bool,
}

fn hash(partition: &str, url: &Url) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in partition.bytes().chain([0]).chain(url.as_str().bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn header(headers: &HeaderMap, name: hyper::header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

struct Directives {
    no_store: bool,
    no_cache: bool,
    max_age: Option<u64>,
}

fn directives(headers: &HeaderMap) -> Directives {
    let mut found = Directives {
        no_store: false,
        no_cache: header(headers, PRAGMA).is_some_and(|pragma| pragma.contains("no-cache")),
        max_age: None,
    };
    for value in headers.get_all(CACHE_CONTROL) {
        let Ok(value) = value.to_str() else {
            continue;
        };
        for directive in value.split(',') {
            let (name, argument) = directive.split_once('=').unwrap_or((directive, ""));
            match name.trim().to_ascii_lowercase().as_str() {
                "no-store" => found.no_store = true,
                "no-cache" => found.no_cache = true,
                "max-age" => {
                    found.max_age = argument
                        .trim()
                        .trim_matches('"')
                        .parse()
                        .ok()
                        .or(found.max_age);
                }
                _ => {}
            }
        }
    }
    found
}

fn storable_status(status: u16) -> bool {
    matches!(status, 200 | 203 | 204 | 300 | 404 | 405 | 410 | 414 | 501)
}

impl Cache {
    pub(crate) fn memory() -> Self {
        Self {
            folder: None,
            entries: HashMap::new(),
            total: 0,
            clock: 0,
            changed: false,
        }
    }

    pub(crate) fn on_disk(folder: PathBuf) -> Self {
        let mut cache = Self::memory();
        let index = std::fs::read_to_string(folder.join(INDEX)).unwrap_or_default();
        for line in index.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let [
                url,
                partition,
                status,
                content_type,
                stored,
                lifetime,
                etag,
                last_modified,
                size,
                used,
            ] = fields.as_slice()
            else {
                continue;
            };
            let (Ok(url), Ok(status), Ok(stored), Ok(lifetime), Ok(size), Ok(used)) = (
                Url::parse(url),
                status.parse(),
                stored.parse(),
                lifetime.parse(),
                size.parse::<usize>(),
                used.parse::<u64>(),
            ) else {
                continue;
            };
            let optional = |text: &str| (text != "-").then(|| text.to_owned());
            let entry = Entry {
                partition: (*partition).to_owned(),
                status,
                content_type: (*content_type).to_owned(),
                stored,
                lifetime,
                etag: optional(etag),
                last_modified: optional(last_modified),
                size,
                used,
                body: None,
                url,
            };
            cache.clock = cache.clock.max(used);
            cache.total += size;
            cache
                .entries
                .insert(hash(&entry.partition, &entry.url), entry);
        }
        cache.folder = Some(folder);
        cache.evict();
        cache
    }

    fn limit(&self) -> usize {
        match self.folder {
            Some(_) => DISK_LIMIT,
            None => MEMORY_LIMIT,
        }
    }

    pub(crate) fn lookup(&mut self, partition: &str, url: &Url, now: u64) -> Lookup {
        let key = hash(partition, url);
        self.clock += 1;
        let clock = self.clock;
        let Some(entry) = self.entries.get_mut(&key) else {
            return Lookup::Miss;
        };
        if entry.url != *url || entry.partition != partition {
            return Lookup::Miss;
        }
        entry.used = clock;
        self.changed = true;
        let fresh = now.saturating_sub(entry.stored) < entry.lifetime;
        if !fresh {
            if entry.etag.is_none() && entry.last_modified.is_none() {
                return Lookup::Miss;
            }
            return Lookup::Stale {
                etag: entry.etag.clone(),
                last_modified: entry.last_modified.clone(),
            };
        }
        match self.body(key) {
            Some(response) => Lookup::Fresh(response),
            None => Lookup::Miss,
        }
    }

    fn body(&mut self, key: u64) -> Option<Response> {
        let entry = self.entries.get(&key)?;
        let body = match (&entry.body, &self.folder) {
            (Some(body), _) => body.clone(),
            (None, Some(folder)) => std::fs::read(folder.join(format!("{key:016x}"))).ok()?,
            (None, None) => return None,
        };
        if body.len() != entry.size {
            self.remove(key);
            return None;
        }
        Some(Response {
            url: entry.url.clone(),
            status: entry.status,
            content_type: ContentType::parse(entry.content_type.as_bytes()),
            body,
        })
    }

    pub(crate) fn revalidated(
        &mut self,
        partition: &str,
        url: &Url,
        headers: &HeaderMap,
        now: u64,
    ) -> Option<Response> {
        let key = hash(partition, url);
        let lifetime = lifetime(headers, now);
        let entry = self.entries.get_mut(&key)?;
        entry.stored = now;
        entry.lifetime = lifetime.unwrap_or(0);
        if let Some(etag) = header(headers, ETAG) {
            entry.etag = Some(etag.to_owned());
        }
        self.changed = true;
        self.body(key)
    }

    pub(crate) fn store(
        &mut self,
        partition: &str,
        response: &Response,
        headers: &HeaderMap,
        cookie_sent: bool,
        now: u64,
    ) {
        let found = directives(headers);
        let vary_blocks = headers.get_all(VARY).iter().any(|value| {
            value.to_str().map_or(true, |value| {
                value.split(',').any(|name| {
                    let name = name.trim().to_ascii_lowercase();
                    name == "*" || (name == "cookie" && cookie_sent)
                })
            })
        });
        let etag = header(headers, ETAG).map(str::to_owned);
        let last_modified = header(headers, LAST_MODIFIED).map(str::to_owned);
        let lifetime = match found.no_cache {
            true => 0,
            false => lifetime(headers, now).unwrap_or(0),
        };
        let size = response.body.len();
        let useful = lifetime > 0 || etag.is_some() || last_modified.is_some();
        let key = hash(partition, &response.url);
        if found.no_store
            || vary_blocks
            || !useful
            || !storable_status(response.status)
            || size > ENTRY_LIMIT
            || !matches!(response.url.scheme(), "http" | "https")
        {
            self.remove(key);
            return;
        }
        self.remove(key);
        self.clock += 1;
        let body = match &self.folder {
            Some(folder) => {
                let written = std::fs::create_dir_all(folder).and_then(|()| {
                    std::fs::write(folder.join(format!("{key:016x}")), &response.body)
                });
                if written.is_err() {
                    return;
                }
                None
            }
            None => Some(response.body.clone()),
        };
        self.entries.insert(
            key,
            Entry {
                url: response.url.clone(),
                partition: partition.to_owned(),
                status: response.status,
                content_type: header(headers, CONTENT_TYPE).unwrap_or_default().to_owned(),
                stored: now.saturating_sub(
                    header(headers, AGE)
                        .and_then(|age| age.parse().ok())
                        .unwrap_or(0),
                ),
                lifetime,
                etag,
                last_modified,
                size,
                used: self.clock,
                body,
            },
        );
        self.total += size;
        self.changed = true;
        self.evict();
    }

    fn remove(&mut self, key: u64) {
        if let Some(entry) = self.entries.remove(&key) {
            self.total = self.total.saturating_sub(entry.size);
            if let Some(folder) = &self.folder {
                let _ = std::fs::remove_file(folder.join(format!("{key:016x}")));
            }
            self.changed = true;
        }
    }

    fn evict(&mut self) {
        while self.total > self.limit() {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.used)
                .map(|(key, _)| *key)
            else {
                return;
            };
            self.remove(oldest);
        }
    }

    pub(crate) fn persist(&mut self) {
        if !std::mem::take(&mut self.changed) {
            return;
        }
        let Some(folder) = &self.folder else {
            return;
        };
        let text: String = self
            .entries
            .values()
            .filter(|entry| {
                clean(&entry.content_type)
                    && entry.etag.as_deref().is_none_or(clean)
                    && entry.last_modified.as_deref().is_none_or(clean)
            })
            .map(|entry| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    entry.url,
                    entry.partition,
                    entry.status,
                    entry.content_type,
                    entry.stored,
                    entry.lifetime,
                    entry.etag.as_deref().unwrap_or("-"),
                    entry.last_modified.as_deref().unwrap_or("-"),
                    entry.size,
                    entry.used,
                )
            })
            .collect();
        let _ = crate::storage::replace(&folder.join(INDEX), &text);
    }
}

fn clean(text: &str) -> bool {
    !text.contains(['\t', '\n', '\r'])
}

fn lifetime(headers: &HeaderMap, now: u64) -> Option<u64> {
    let found = directives(headers);
    if let Some(max_age) = found.max_age {
        return Some(max_age);
    }
    let date = header(headers, DATE)
        .and_then(parse_cookie_date)
        .unwrap_or(now);
    if let Some(expires) = header(headers, EXPIRES) {
        return Some(parse_cookie_date(expires).map_or(0, |expires| expires.saturating_sub(date)));
    }
    let modified = header(headers, LAST_MODIFIED).and_then(parse_cookie_date)?;
    Some((date.saturating_sub(modified) / 10).min(HEURISTIC_LIMIT))
}
