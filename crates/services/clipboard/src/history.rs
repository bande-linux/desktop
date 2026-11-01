use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;

pub type Contents = Vec<(String, Vec<u8>)>;

pub const KEPT_ITEMS: usize = 100;
pub const KEPT_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const MAX_ITEM_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
const SUMMARY_CHARACTERS: usize = 2000;
const TEXT_MIMES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
    "TEXT",
];
const URI_LIST: &str = "text/uri-list";
const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
const CONCEALED: &str = "application/x-nspasteboard-concealed-type";

pub fn is_sensitive(contents: &Contents) -> bool {
    contents.iter().any(|(mime, bytes)| {
        mime == CONCEALED || (mime == PASSWORD_HINT && bytes.trim_ascii() == b"secret")
    })
}

pub fn privacy_hint_unread(offered: &[String], contents: &Contents) -> bool {
    offered.iter().any(|mime| {
        (mime == PASSWORD_HINT || mime == CONCEALED)
            && !contents.iter().any(|(read, _)| read == mime)
    })
}

pub fn text_of(contents: &Contents) -> Option<String> {
    TEXT_MIMES.iter().find_map(|wanted| {
        contents
            .iter()
            .find(|(mime, _)| mime.eq_ignore_ascii_case(wanted))
            .and_then(|(_, bytes)| String::from_utf8(bytes.clone()).ok())
    })
}

pub fn summary(contents: &Contents) -> String {
    let text = contents
        .iter()
        .find(|(mime, _)| mime == URI_LIST)
        .and_then(|(_, bytes)| String::from_utf8(bytes.clone()).ok())
        .or_else(|| text_of(contents))
        .unwrap_or_default();
    text.chars().take(SUMMARY_CHARACTERS).collect()
}

fn size(contents: &Contents) -> usize {
    contents
        .iter()
        .map(|(mime, bytes)| mime.len() + bytes.len())
        .sum()
}

fn same(a: &Contents, b: &Contents) -> bool {
    let key = |contents: &Contents| {
        text_of(contents).map(String::into_bytes).or_else(|| {
            contents
                .iter()
                .find(|(mime, _)| mime.starts_with("image/"))
                .map(|(_, bytes)| bytes.clone())
        })
    };
    match (key(a), key(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub copied: u64,
    pub contents: Contents,
}

impl Entry {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = self.copied.to_le_bytes().to_vec();
        for (mime, data) in &self.contents {
            bytes.extend_from_slice(&(mime.len() as u32).to_le_bytes());
            bytes.extend_from_slice(mime.as_bytes());
            bytes.extend_from_slice(&(data.len() as u64).to_le_bytes());
            bytes.extend_from_slice(data);
        }
        bytes
    }

    fn decode(id: u64, bytes: &[u8]) -> Option<Self> {
        let (copied, mut rest) = bytes.split_first_chunk::<8>()?;
        let mut contents = Vec::new();
        while !rest.is_empty() {
            let (length, after) = rest.split_first_chunk::<4>()?;
            let (mime, after) = after.split_at_checked(u32::from_le_bytes(*length) as usize)?;
            let (length, after) = after.split_first_chunk::<8>()?;
            let length = usize::try_from(u64::from_le_bytes(*length)).ok()?;
            let (data, after) = after.split_at_checked(length)?;
            contents.push((String::from_utf8(mime.to_vec()).ok()?, data.to_vec()));
            rest = after;
        }
        Some(Self {
            id,
            copied: u64::from_le_bytes(*copied),
            contents,
        })
    }
}

#[derive(Debug, Default)]
pub struct History {
    dir: Option<PathBuf>,
    entries: Vec<Entry>,
    next_id: u64,
}

impl History {
    pub fn load(dir: PathBuf, now: u64) -> Self {
        let mut entries: Vec<Entry> = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|file| {
                let id = u64::from_str_radix(file.file_name().to_str()?, 16).ok()?;
                Entry::decode(id, &fs::read(file.path()).ok()?)
            })
            .collect();
        entries.sort_by(|a, b| b.copied.cmp(&a.copied).then(b.id.cmp(&a.id)));
        let next_id = entries.iter().map(|entry| entry.id + 1).max().unwrap_or(1);
        let mut history = Self {
            dir: Some(dir),
            entries,
            next_id,
        };
        history.trim(now);
        history
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    pub fn record(&mut self, contents: Contents, now: u64) -> bool {
        if contents.is_empty() || is_sensitive(&contents) || size(&contents) > MAX_ITEM_BYTES {
            return false;
        }
        let stale: Vec<u64> = self
            .entries
            .iter()
            .filter(|entry| same(&entry.contents, &contents))
            .map(|entry| entry.id)
            .collect();
        for id in stale {
            self.remove(id);
        }
        let entry = Entry {
            id: self.next_id,
            copied: now,
            contents,
        };
        self.next_id += 1;
        if let Err(error) = self.write(&entry) {
            tracing::warn!("cannot keep a copy in the history: {error}");
        }
        self.entries.insert(0, entry);
        self.trim(now);
        true
    }

    pub fn promote(&mut self, id: u64, now: u64) -> Option<Contents> {
        let position = self.entries.iter().position(|entry| entry.id == id)?;
        let mut entry = self.entries.remove(position);
        entry.copied = now;
        if let Err(error) = self.write(&entry) {
            tracing::warn!("cannot update the history: {error}");
        }
        let contents = entry.contents.clone();
        self.entries.insert(0, entry);
        Some(contents)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        if let Some(path) = self.path(id) {
            let _ = fs::remove_file(path);
        }
        self.entries.len() != before
    }

    pub fn clear(&mut self) {
        let ids: Vec<u64> = self.entries.iter().map(|entry| entry.id).collect();
        for id in ids {
            self.remove(id);
        }
    }

    pub fn trim(&mut self, now: u64) {
        let mut total = 0;
        let doomed: Vec<u64> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(index, entry)| {
                total += size(&entry.contents);
                *index >= KEPT_ITEMS
                    || now.saturating_sub(entry.copied) > KEPT_SECONDS
                    || total > MAX_BYTES
            })
            .map(|(_, entry)| entry.id)
            .collect();
        for id in doomed {
            self.remove(id);
        }
    }

    fn path(&self, id: u64) -> Option<PathBuf> {
        self.dir.as_ref().map(|dir| dir.join(format!("{id:016x}")))
    }

    fn write(&self, entry: &Entry) -> io::Result<()> {
        let (Some(dir), Some(path)) = (&self.dir, self.path(entry.id)) else {
            return Ok(());
        };
        DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let partial = path.with_extension("partial");
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&partial)?
            .write_all(&entry.encode())?;
        fs::rename(partial, path)
    }
}
