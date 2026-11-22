use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bande_mail_core::accounts::AccountId;
use bande_mail_core::error::Result;
use bande_toolkit::ui::RichText;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::note::{self, Note};

pub type FolderId = i64;
pub type NoteId = i64;

pub const LOCAL: AccountId = 0;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS folders (
    id INTEGER PRIMARY KEY,
    account INTEGER NOT NULL,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    depth INTEGER NOT NULL DEFAULT 0,
    uid_validity INTEGER,
    uid_next INTEGER,
    present INTEGER,
    modseq INTEGER,
    UNIQUE (account, path)
);
CREATE TABLE IF NOT EXISTS notes (
    id INTEGER PRIMARY KEY,
    folder INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uuid TEXT NOT NULL,
    uid INTEGER,
    created INTEGER NOT NULL,
    modified INTEGER NOT NULL,
    title TEXT NOT NULL,
    snippet TEXT NOT NULL,
    body TEXT NOT NULL,
    text TEXT NOT NULL,
    extras INTEGER NOT NULL DEFAULT 0,
    dirty INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS notes_by_folder ON notes (folder, modified);
CREATE TABLE IF NOT EXISTS removals (
    id INTEGER PRIMARY KEY,
    folder INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    uid INTEGER NOT NULL,
    uid_validity INTEGER NOT NULL,
    trash INTEGER NOT NULL
);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: FolderId,
    pub account: AccountId,
    pub path: String,
    pub name: String,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FolderState {
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
    pub present: Option<u32>,
    pub modseq: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub id: NoteId,
    pub title: String,
    pub snippet: String,
    pub modified: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub id: NoteId,
    pub folder: FolderId,
    pub uid: Option<u32>,
    pub note: Note,
    pub dirty: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Deleted {
    stored: Stored,
    removal: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removal {
    pub id: i64,
    pub folder: FolderId,
    pub uid: u32,
    pub uid_validity: u32,
    pub trash: bool,
}

pub struct Store {
    connection: Connection,
}

pub fn store_path() -> PathBuf {
    bande_xdg::data_home()
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("bande-notes/notes.sqlite3")
}

fn stored_from(row: &Row<'_>) -> rusqlite::Result<Stored> {
    let body: String = row.get(5)?;
    Ok(Stored {
        id: row.get(0)?,
        folder: row.get(1)?,
        uid: row.get(3)?,
        note: Note {
            uuid: row.get(2)?,
            created: row.get(4)?,
            modified: row.get(6)?,
            rich: note::from_html(&body),
            extras: row.get(7)?,
        },
        dirty: row.get(8)?,
    })
}

const STORED: &str =
    "SELECT id, folder, uuid, uid, created, body, modified, extras, dirty FROM notes";

fn escaped_like(text: &str) -> String {
    let mut escaped = String::from("%");
    for character in text.chars() {
        if matches!(character, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('%');
    escaped
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.execute_batch(SCHEMA)?;
        connection.execute(
            "INSERT OR IGNORE INTO folders (account, path, name) VALUES (?1, '', '')",
            [LOCAL],
        )?;
        Ok(Self { connection })
    }

    pub fn folders(&self) -> Result<Vec<Folder>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, account, path, name, depth FROM folders ORDER BY account, path")?;
        Ok(statement
            .query_map([], |row| {
                Ok(Folder {
                    id: row.get(0)?,
                    account: row.get(1)?,
                    path: row.get(2)?,
                    name: row.get(3)?,
                    depth: row.get::<_, i64>(4)?.max(0) as usize,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn folder(&self, id: FolderId) -> Result<Option<Folder>> {
        Ok(self.folders()?.into_iter().find(|folder| folder.id == id))
    }

    pub fn keep_accounts(&self, accounts: &[AccountId]) -> Result<()> {
        let kept = accounts
            .iter()
            .map(AccountId::to_string)
            .collect::<Vec<_>>()
            .join(",");
        self.connection.execute(
            &format!("DELETE FROM folders WHERE account != {LOCAL} AND account NOT IN ({kept})"),
            [],
        )?;
        Ok(())
    }

    pub fn replace_folders(
        &mut self,
        account: AccountId,
        folders: &[(String, String, usize)],
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let known = {
            let mut statement =
                transaction.prepare("SELECT path FROM folders WHERE account = ?1")?;
            statement
                .query_map([account], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for path in known
            .iter()
            .filter(|path| !folders.iter().any(|(known, _, _)| known == *path))
        {
            transaction.execute(
                "DELETE FROM folders WHERE account = ?1 AND path = ?2",
                params![account, path],
            )?;
        }
        for (path, name, depth) in folders {
            transaction.execute(
                "INSERT INTO folders (account, path, name, depth) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (account, path) DO UPDATE SET name = ?3, depth = ?4",
                params![account, path, name, *depth as i64],
            )?;
        }
        Ok(transaction.commit()?)
    }

    pub fn folder_state(&self, folder: FolderId) -> Result<FolderState> {
        Ok(self.connection.query_row(
            "SELECT uid_validity, uid_next, present, modseq FROM folders WHERE id = ?1",
            [folder],
            |row| {
                Ok(FolderState {
                    uid_validity: row.get(0)?,
                    uid_next: row.get(1)?,
                    present: row.get(2)?,
                    modseq: row.get::<_, Option<i64>>(3)?.map(|modseq| modseq as u64),
                })
            },
        )?)
    }

    pub fn set_folder_state(&self, folder: FolderId, state: FolderState) -> Result<()> {
        self.connection.execute(
            "UPDATE folders SET uid_validity = ?2, uid_next = ?3, present = ?4, modseq = ?5 \
             WHERE id = ?1",
            params![
                folder,
                state.uid_validity,
                state.uid_next,
                state.present,
                state.modseq.map(|modseq| modseq as i64)
            ],
        )?;
        Ok(())
    }

    pub fn summaries(&self, folder: FolderId, search: &str) -> Result<Vec<Summary>> {
        let mut statement = self.connection.prepare(
            "SELECT id, title, snippet, modified FROM notes \
             WHERE folder = ?1 AND (text LIKE ?2 ESCAPE '\\') ORDER BY modified DESC, id DESC",
        )?;
        Ok(statement
            .query_map(params![folder, escaped_like(search.trim())], |row| {
                Ok(Summary {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    snippet: row.get(2)?,
                    modified: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn stored(&self, id: NoteId) -> Result<Option<Stored>> {
        Ok(self
            .connection
            .query_row(&format!("{STORED} WHERE id = ?1"), [id], stored_from)
            .optional()?)
    }

    fn write_row(&self, id: NoteId, note: &Note, uid: Option<u32>, dirty: bool) -> Result<()> {
        self.connection.execute(
            "UPDATE notes SET uuid = ?2, uid = ?3, created = ?4, modified = ?5, title = ?6, \
             snippet = ?7, body = ?8, text = ?9, extras = ?10, dirty = ?11 WHERE id = ?1",
            params![
                id,
                note.uuid,
                uid,
                note.created,
                note.modified,
                note::title(&note.rich),
                note::snippet(&note.rich),
                note::to_html(&note.rich),
                note.rich.text,
                note.extras,
                dirty,
            ],
        )?;
        Ok(())
    }

    pub fn insert(
        &self,
        folder: FolderId,
        note: &Note,
        uid: Option<u32>,
        dirty: bool,
    ) -> Result<NoteId> {
        self.connection.execute(
            "INSERT INTO notes (folder, uuid, created, modified, title, snippet, body, text) \
             VALUES (?1, '', 0, 0, '', '', '', '')",
            [folder],
        )?;
        let id = self.connection.last_insert_rowid();
        self.write_row(id, note, uid, dirty)?;
        Ok(id)
    }

    pub fn create(&self, folder: FolderId, now: i64) -> Result<NoteId> {
        let note = Note {
            uuid: note::new_uuid(),
            created: now,
            modified: now,
            rich: RichText::default(),
            extras: false,
        };
        self.insert(folder, &note, None, true)
    }

    pub fn edit(&self, id: NoteId, rich: &RichText, now: i64) -> Result<()> {
        let Some(stored) = self.stored(id)? else {
            return Ok(());
        };
        let note = Note {
            rich: rich.clone(),
            modified: now,
            extras: false,
            ..stored.note
        };
        self.write_row(id, &note, stored.uid, true)
    }

    fn queue_removal(&self, stored: &Stored, trash: bool) -> Result<Option<i64>> {
        let (Some(uid), Some(validity)) =
            (stored.uid, self.folder_state(stored.folder)?.uid_validity)
        else {
            return Ok(None);
        };
        self.connection.execute(
            "INSERT INTO removals (folder, uid, uid_validity, trash) VALUES (?1, ?2, ?3, ?4)",
            params![stored.folder, uid, validity, trash],
        )?;
        Ok(Some(self.connection.last_insert_rowid()))
    }

    pub fn delete(&self, id: NoteId) -> Result<Option<Deleted>> {
        let Some(stored) = self.stored(id)? else {
            return Ok(None);
        };
        let removal = self.queue_removal(&stored, true)?;
        self.connection
            .execute("DELETE FROM notes WHERE id = ?1", [id])?;
        Ok(Some(Deleted { stored, removal }))
    }

    pub fn restore(&self, deleted: &Deleted) -> Result<NoteId> {
        let stored = &deleted.stored;
        let still_queued = match deleted.removal {
            Some(removal) => {
                self.connection
                    .execute("DELETE FROM removals WHERE id = ?1", [removal])?
                    > 0
            }
            None => false,
        };
        let (uid, dirty) = match still_queued || deleted.removal.is_none() {
            true => (stored.uid, stored.dirty),
            false => (None, true),
        };
        self.insert(stored.folder, &stored.note, uid, dirty)
    }

    pub fn move_note(&self, id: NoteId, target: FolderId) -> Result<Option<NoteId>> {
        let Some(stored) = self.stored(id)? else {
            return Ok(None);
        };
        if stored.folder == target {
            return Ok(Some(id));
        }
        self.queue_removal(&stored, false)?;
        self.connection.execute(
            "UPDATE notes SET folder = ?2, uid = NULL, dirty = 1 WHERE id = ?1",
            params![id, target],
        )?;
        Ok(Some(id))
    }

    pub fn known(&self, folder: FolderId) -> Result<Vec<(u32, NoteId, bool)>> {
        let mut statement = self.connection.prepare(
            "SELECT uid, id, dirty FROM notes WHERE folder = ?1 AND uid IS NOT NULL ORDER BY uid",
        )?;
        Ok(statement
            .query_map([folder], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn forget_clean(&self, folder: FolderId, uids: &[u32]) -> Result<()> {
        for uid in uids {
            self.connection.execute(
                "DELETE FROM notes WHERE folder = ?1 AND uid = ?2 AND dirty = 0",
                params![folder, uid],
            )?;
        }
        Ok(())
    }

    pub fn start_over(&self, folder: FolderId) -> Result<()> {
        self.connection.execute(
            "DELETE FROM notes WHERE folder = ?1 AND dirty = 0",
            [folder],
        )?;
        self.connection
            .execute("UPDATE notes SET uid = NULL WHERE folder = ?1", [folder])?;
        self.connection
            .execute("DELETE FROM removals WHERE folder = ?1", [folder])?;
        Ok(())
    }

    pub fn arrived(&self, folder: FolderId, uid: u32, note: &Note) -> Result<()> {
        let replaced: Option<NoteId> = self
            .connection
            .query_row(
                "SELECT id FROM notes WHERE folder = ?1 AND uuid = ?2 AND dirty = 0 \
                 AND (uid IS NULL OR uid < ?3) ORDER BY id LIMIT 1",
                params![folder, note.uuid, uid],
                |row| row.get(0),
            )
            .optional()?;
        match replaced {
            Some(id) => self.write_row(id, note, Some(uid), false),
            None => self.insert(folder, note, Some(uid), false).map(drop),
        }
    }

    pub fn uuid_taken(&self, folder: FolderId, uuid: &str, except: NoteId) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM notes WHERE folder = ?1 AND uuid = ?2 AND id != ?3)",
            params![folder, uuid, except],
            |row| row.get(0),
        )?)
    }

    pub fn dirty(&self, folder: FolderId) -> Result<Vec<Stored>> {
        let mut statement = self.connection.prepare(&format!(
            "{STORED} WHERE folder = ?1 AND dirty = 1 ORDER BY id"
        ))?;
        Ok(statement
            .query_map([folder], stored_from)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn uploaded(&self, id: NoteId, uuid: &str, uid: Option<u32>, modified: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE notes SET uuid = ?2, uid = ?3, dirty = (modified != ?4) WHERE id = ?1",
            params![id, uuid, uid, modified],
        )?;
        Ok(())
    }

    pub fn removals(&self, folder: FolderId) -> Result<Vec<Removal>> {
        let mut statement = self.connection.prepare(
            "SELECT id, folder, uid, uid_validity, trash FROM removals WHERE folder = ?1 ORDER BY id",
        )?;
        Ok(statement
            .query_map([folder], |row| {
                Ok(Removal {
                    id: row.get(0)?,
                    folder: row.get(1)?,
                    uid: row.get(2)?,
                    uid_validity: row.get(3)?,
                    trash: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn removed(&self, removal: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM removals WHERE id = ?1", [removal])?;
        Ok(())
    }
}
