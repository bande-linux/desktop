use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Row, params};

use crate::error::{Error, Result};
use crate::net::{Security, Server};

pub type AccountId = i64;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS accounts (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    user TEXT NOT NULL,
    imap_host TEXT NOT NULL,
    imap_port INTEGER NOT NULL,
    imap_security TEXT NOT NULL,
    smtp_host TEXT NOT NULL,
    smtp_port INTEGER NOT NULL,
    smtp_security TEXT NOT NULL,
    signature TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS notes_accounts (
    account INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE
);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub email: String,
    pub user: String,
    pub imap: Server,
    pub smtp: Server,
    pub signature: String,
    pub enabled: bool,
}

pub fn store_path() -> Result<PathBuf> {
    bande_xdg::data_home()
        .map(|home| home.join("bande-mail/mail.sqlite3"))
        .ok_or(Error::NoDataFolder)
}

pub fn open_store() -> Result<Connection> {
    open(&store_path()?)
}

pub fn open(path: &Path) -> Result<Connection> {
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
    Ok(connection)
}

fn security(name: String) -> Security {
    Security::from_name(&name).unwrap_or(Security::Tls)
}

fn account_from(row: &Row<'_>) -> rusqlite::Result<Account> {
    Ok(Account {
        id: row.get(0)?,
        name: row.get(1)?,
        email: row.get(2)?,
        user: row.get(3)?,
        imap: Server {
            host: row.get(4)?,
            port: row.get(5)?,
            security: security(row.get(6)?),
        },
        smtp: Server {
            host: row.get(7)?,
            port: row.get(8)?,
            security: security(row.get(9)?),
        },
        signature: row.get(10)?,
        enabled: row.get(11)?,
    })
}

pub fn list(connection: &Connection) -> Result<Vec<Account>> {
    let mut statement = connection.prepare(
        "SELECT id, name, email, user, imap_host, imap_port, imap_security, \
         smtp_host, smtp_port, smtp_security, signature, enabled FROM accounts ORDER BY id",
    )?;
    let accounts = statement
        .query_map([], account_from)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(accounts)
}

pub fn add(connection: &Connection, account: &Account) -> Result<AccountId> {
    connection.execute(
        "INSERT INTO accounts (name, email, user, imap_host, imap_port, imap_security, \
         smtp_host, smtp_port, smtp_security, signature) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            account.name,
            account.email,
            account.user,
            account.imap.host,
            account.imap.port,
            account.imap.security.name(),
            account.smtp.host,
            account.smtp.port,
            account.smtp.security.name(),
            account.signature,
        ],
    )?;
    Ok(connection.last_insert_rowid())
}

pub fn notes(connection: &Connection) -> Result<Vec<AccountId>> {
    let mut statement =
        connection.prepare("SELECT account FROM notes_accounts ORDER BY account")?;
    let ids = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

pub fn set_notes(connection: &Connection, account: AccountId, used: bool) -> Result<()> {
    match used {
        true => connection.execute(
            "INSERT OR IGNORE INTO notes_accounts (account) VALUES (?1)",
            [account],
        )?,
        false => connection.execute("DELETE FROM notes_accounts WHERE account = ?1", [account])?,
    };
    Ok(())
}
