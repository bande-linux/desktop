use std::path::Path;
use std::time::Duration;

use bande_search::{Restored, Stamp};
use rusqlite::{Connection, OptionalExtension, Result, params};

use crate::background::Offered;
use crate::tokens::{indexed, stands_alone, words};

const DATABASE: &str = "index.sqlite";
const SCHEMA_VERSION: i32 = 1;
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);
const SCHEMA: &str = "
CREATE TABLE entries (
    id INTEGER PRIMARY KEY,
    parent INTEGER NOT NULL,
    name TEXT NOT NULL,
    folder INTEGER NOT NULL,
    inode INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    size INTEGER NOT NULL,
    imported INTEGER NOT NULL
);
CREATE TABLE texts (
    id INTEGER PRIMARY KEY,
    file INTEGER UNIQUE,
    key TEXT UNIQUE,
    domain TEXT,
    kind TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    target TEXT NOT NULL DEFAULT ''
);
CREATE INDEX texts_by_domain ON texts (domain);
CREATE VIRTUAL TABLE words USING fts5 (
    body,
    content = '',
    contentless_delete = 1,
    tokenize = \"unicode61 remove_diacritics 2 categories 'L* N* Co M*'\"
);
CREATE TRIGGER texts_removed AFTER DELETE ON texts BEGIN
    DELETE FROM words WHERE rowid = old.id;
END;
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub parent: u32,
    pub name: String,
    pub folder: bool,
    pub stamp: Stamp,
}

pub enum Hit {
    File { id: u32, relevance: f32 },
    Item(Item),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub domain: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub target: String,
    pub relevance: f32,
}

pub struct Store {
    connection: Connection,
}

pub struct Writer {
    connection: Connection,
}

fn connect(folder: &Path) -> Result<Connection> {
    let connection = Connection::open(folder.join(DATABASE))?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(connection)
}

fn schema_version(connection: &Connection) -> Result<i32> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

pub fn open(folder: &Path) -> Result<(Store, Writer)> {
    let current = connect(folder).and_then(|connection| schema_version(&connection));
    if current.ok() != Some(SCHEMA_VERSION) {
        wipe(folder);
        let created = connect(folder)?;
        created.execute_batch(SCHEMA)?;
        created.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    let store = Store {
        connection: connect(folder)?,
    };
    let writer = Writer {
        connection: connect(folder)?,
    };
    Ok((store, writer))
}

fn wipe(folder: &Path) {
    let Ok(files) = std::fs::read_dir(folder) else {
        return;
    };
    for file in files.flatten() {
        if let Err(error) = std::fs::remove_file(file.path()) {
            tracing::warn!("cannot clear the old search index: {error}");
        }
    }
}

impl Store {
    pub fn entries(&self) -> Result<Vec<Restored>> {
        let mut statement = self.connection.prepare(
            "SELECT id, parent, name, folder, inode, modified, size, imported FROM entries",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Restored {
                id: row.get(0)?,
                parent: row.get(1)?,
                name: row.get(2)?,
                folder: row.get(3)?,
                stamp: Stamp {
                    inode: row.get::<_, i64>(4)?.cast_unsigned(),
                    modified: row.get(5)?,
                    size: row.get::<_, i64>(6)?.cast_unsigned(),
                    imported: row.get(7)?,
                },
            })
        })?;
        rows.collect()
    }

    pub fn stored_title(&self, key: &str) -> Option<String> {
        self.connection
            .query_row("SELECT title FROM texts WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .inspect_err(|error| tracing::warn!("cannot read the search index: {error}"))
            .ok()
            .flatten()
    }

    pub fn search(&self, text: &str, limit: usize, within: Option<&[bool]>) -> Result<Vec<Hit>> {
        let Some(query) = content_query(text) else {
            return Ok(Vec::new());
        };
        let mut statement = self.connection.prepare_cached(
            "SELECT texts.file, texts.domain, texts.kind, texts.title, texts.detail, texts.target,
                    -words.rank
             FROM words JOIN texts ON texts.id = words.rowid
             WHERE words MATCH ?1 ORDER BY words.rank",
        )?;
        let mut rows = statement.query([query])?;
        let mut hits = Vec::new();
        while hits.len() < limit.max(1)
            && let Some(row) = rows.next()?
        {
            let relevance = row.get::<_, f64>(6)? as f32;
            let file: Option<u32> = row.get(0)?;
            match (file, within) {
                (Some(id), Some(allowed))
                    if !usize::try_from(id)
                        .ok()
                        .and_then(|id| allowed.get(id).copied())
                        .unwrap_or(false) => {}
                (Some(id), _) => hits.push(Hit::File { id, relevance }),
                (None, Some(_)) => {}
                (None, None) => {
                    let kind: String = row.get(2)?;
                    if kind.is_empty() {
                        continue;
                    }
                    hits.push(Hit::Item(Item {
                        domain: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        kind,
                        title: row.get(3)?,
                        detail: row.get(4)?,
                        target: row.get(5)?,
                        relevance,
                    }));
                }
            }
        }
        Ok(hits)
    }
}

impl Writer {
    fn begin(&self) -> Result<()> {
        match self.connection.is_autocommit() {
            true => self.connection.execute_batch("BEGIN IMMEDIATE"),
            false => Ok(()),
        }
    }

    pub fn commit(&self) -> Result<()> {
        match self.connection.is_autocommit() {
            true => Ok(()),
            false => self.connection.execute_batch("COMMIT"),
        }
    }

    pub fn put_entry(&self, entry: &Entry) -> Result<()> {
        self.begin()?;
        self.connection
            .prepare_cached(
                "INSERT OR REPLACE INTO entries
                 (id, parent, name, folder, inode, modified, size, imported)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute(params![
                entry.id,
                entry.parent,
                entry.name,
                entry.folder,
                entry.stamp.inode.cast_signed(),
                entry.stamp.modified,
                entry.stamp.size.cast_signed(),
                entry.stamp.imported,
            ])?;
        Ok(())
    }

    pub fn remove_entry(&self, id: u32) -> Result<()> {
        self.begin()?;
        self.connection
            .prepare_cached("DELETE FROM entries WHERE id = ?1")?
            .execute([id])?;
        self.forget_text(id)
    }

    pub fn forget_text(&self, file: u32) -> Result<()> {
        self.begin()?;
        self.connection
            .prepare_cached("DELETE FROM texts WHERE file = ?1")?
            .execute([file])?;
        Ok(())
    }

    pub fn put_text(&self, file: u32, body: &str) -> Result<()> {
        self.forget_text(file)?;
        self.connection
            .prepare_cached("INSERT INTO texts (file) VALUES (?1)")?
            .execute([file])?;
        self.add_words(body)
    }

    pub fn put_item(&self, item: &Offered) -> Result<()> {
        self.forget_item(&item.key)?;
        self.connection
            .prepare_cached(
                "INSERT INTO texts (key, domain, kind, title, detail, target)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                item.key,
                item.domain,
                item.kind,
                item.title,
                item.detail,
                item.target,
            ])?;
        self.add_words(&format!("{}\n{}", item.title, item.body))
    }

    fn add_words(&self, body: &str) -> Result<()> {
        self.connection
            .prepare_cached("INSERT INTO words (rowid, body) VALUES (?1, ?2)")?
            .execute(params![self.connection.last_insert_rowid(), indexed(body)])?;
        Ok(())
    }

    pub fn forget_item(&self, key: &str) -> Result<()> {
        self.begin()?;
        self.connection
            .prepare_cached("DELETE FROM texts WHERE key = ?1")?
            .execute([key])?;
        Ok(())
    }

    pub fn forget_domain(&self, domain: &str) -> Result<()> {
        self.begin()?;
        self.connection
            .prepare_cached("DELETE FROM texts WHERE domain = ?1")?
            .execute([domain])?;
        Ok(())
    }

    pub fn compact(&self) -> Result<()> {
        self.commit()?;
        self.connection
            .execute_batch("INSERT INTO words (words) VALUES ('optimize'); VACUUM;")
    }

    pub fn rest(&self) -> Result<()> {
        self.commit()?;
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA shrink_memory;")
    }
}

struct Group {
    words: Vec<String>,
    prefix: bool,
}

fn groups(text: &str) -> Vec<Group> {
    let pieces: Vec<&str> = text.split('"').collect();
    let mut groups = Vec::new();
    for (index, piece) in pieces.iter().enumerate() {
        if index % 2 == 1 {
            groups.push(Group {
                words: words(piece),
                prefix: false,
            });
            continue;
        }
        let typing = index == pieces.len() - 1 && !piece.ends_with(char::is_whitespace);
        let spaced: Vec<&str> = piece.split_whitespace().collect();
        for (at, word) in spaced.iter().enumerate() {
            groups.push(Group {
                words: words(word),
                prefix: typing
                    && at == spaced.len() - 1
                    && !word.chars().last().is_some_and(stands_alone),
            });
        }
    }
    groups.retain(|group| !group.words.is_empty());
    groups
}

fn phrase(group: &Group) -> Option<String> {
    let single = group.words.first().filter(|_| group.words.len() == 1);
    if group.prefix && single.is_some_and(|word| word.chars().count() < 2) {
        return None;
    }
    let star = if group.prefix { "*" } else { "" };
    Some(format!("\"{}\"{star}", group.words.join(" ")))
}

fn content_query(text: &str) -> Option<String> {
    let phrases: Vec<String> = groups(text).iter().filter_map(phrase).collect();
    (!phrases.is_empty()).then(|| phrases.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(store: &Store, text: &str) -> Vec<u32> {
        let mut found: Vec<u32> = store
            .search(text, 10, None)
            .unwrap()
            .into_iter()
            .filter_map(|hit| match hit {
                Hit::File { id, .. } => Some(id),
                Hit::Item(_) => None,
            })
            .collect();
        found.sort();
        found
    }

    #[test]
    fn finds_words_by_prefix_phrase_and_without_accents_or_case() {
        let folder = tempfile::tempdir().unwrap();
        let (store, writer) = open(folder.path()).unwrap();
        writer.put_text(1, "Søren's café on Main Street").unwrap();
        writer.put_text(2, "東京の地図 and the main road").unwrap();
        writer.put_text(3, "Streetcars").unwrap();
        writer.commit().unwrap();
        assert_eq!(found(&store, "CAFE soren"), [1]);
        assert_eq!(found(&store, "street "), [1]);
        assert_eq!(found(&store, "stree"), [1, 3]);
        assert_eq!(found(&store, "\"main street\""), [1]);
        assert_eq!(found(&store, "main road"), [2]);
        assert_eq!(found(&store, "京"), [2]);
        assert_eq!(found(&store, "s"), Vec::<u32>::new());
        writer.put_text(1, "replaced").unwrap();
        writer.remove_entry(3).unwrap();
        writer.commit().unwrap();
        assert_eq!(found(&store, "stree"), Vec::<u32>::new());
    }
}
