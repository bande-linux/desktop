use std::path::{Path, PathBuf};

use bande_dav::{Cached, Pending, RemoteAddressBook, href_for, new_uid, read_or_set_aside};
use serde::{Deserialize, Serialize};

use crate::vcard::{Card, Contact, Kind, parse_cards};

pub const ACCOUNTS_FILE: &str = "accounts.json";
pub const BOOKS_DIR: &str = "books";
pub const LOCAL_ACCOUNT: &str = "local";
const PROGRAM: &str = "bande-contacts";

/// Where Contacts keeps its accounts and the offline copy of every address book.
pub fn data_dir() -> PathBuf {
    bande_xdg::data_home()
        .unwrap_or_else(std::env::temp_dir)
        .join(PROGRAM)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    Local,
    CardDav {
        server: String,
        user: String,
        home: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub kind: AccountKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookInfo {
    pub id: String,
    pub account: String,
    pub url: Option<String>,
    pub name: String,
    pub read_only: bool,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
}

impl BookInfo {
    pub fn remote(account: &str, book: &RemoteAddressBook) -> Self {
        Self {
            id: new_uid(),
            account: account.to_owned(),
            url: Some(book.url.clone()),
            name: book.name.clone(),
            read_only: book.read_only,
            ctag: None,
            sync_token: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    pub href: String,
    pub etag: Option<String>,
    pub data: String,
    pub pending: Option<Pending>,
    #[serde(skip)]
    pub card: Option<Card>,
}

impl Cached for Stored {
    fn fresh(href: String, etag: Option<String>, data: String, pending: Option<Pending>) -> Self {
        Self::new(href, etag, data, pending)
    }

    fn href(&self) -> &str {
        &self.href
    }

    fn data(&self) -> &str {
        &self.data
    }

    fn pending(&self) -> Option<Pending> {
        self.pending
    }

    fn set_etag(&mut self, etag: Option<String>) {
        self.etag = etag;
    }

    fn set_pending(&mut self, pending: Option<Pending>) {
        self.pending = pending;
    }
}

impl Stored {
    pub fn new(href: String, etag: Option<String>, data: String, pending: Option<Pending>) -> Self {
        let card = parse_one(&data);
        Self {
            href,
            etag,
            data,
            pending,
            card,
        }
    }

    pub fn shown(&self) -> Option<&Card> {
        self.card
            .as_ref()
            .filter(|_| self.pending != Some(Pending::Delete))
    }
}

fn parse_one(data: &str) -> Option<Card> {
    parse_cards(data).cards.into_iter().next()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Book {
    pub info: BookInfo,
    pub cards: Vec<Stored>,
}

impl Book {
    pub fn new(info: BookInfo) -> Self {
        Self {
            info,
            cards: Vec::new(),
        }
    }

    pub fn find(&self, uid: &str) -> Option<usize> {
        self.cards
            .iter()
            .position(|stored| stored.shown().is_some_and(|card| card.uid() == uid))
    }

    /// Whether the book keeps its groups as group cards (`KIND:group`) rather than as
    /// `CATEGORIES` on each contact.
    pub fn uses_group_cards(&self) -> bool {
        self.cards
            .iter()
            .filter_map(Stored::shown)
            .any(|card| card.kind() == Kind::Group)
    }

    pub fn for_sync(&self) -> Self {
        Self {
            info: self.info.clone(),
            cards: self
                .cards
                .iter()
                .map(|stored| Stored {
                    href: stored.href.clone(),
                    etag: stored.etag.clone(),
                    data: match stored.pending {
                        Some(_) => stored.data.clone(),
                        None => String::new(),
                    },
                    pending: stored.pending,
                    card: None,
                })
                .collect(),
        }
    }
}

fn book_file(dir: &Path, id: &str) -> PathBuf {
    let name: String = id
        .chars()
        .map(
            |character| match character.is_ascii_alphanumeric() || character == '-' {
                true => character,
                false => '_',
            },
        )
        .collect();
    dir.join(BOOKS_DIR).join(format!("{name}.json"))
}

pub(crate) fn book_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(BOOKS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
}

/// A contact as the window shows it: where it is and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub book: usize,
    pub card: usize,
}

#[derive(Debug, Default)]
pub struct Library {
    dir: Option<PathBuf>,
    pub accounts: Vec<Account>,
    pub books: Vec<Book>,
}

impl Library {
    pub fn load(dir: PathBuf) -> Self {
        let accounts: Vec<Account> =
            read_or_set_aside(&dir.join(ACCOUNTS_FILE)).unwrap_or_default();
        let mut books: Vec<Book> = book_files(&dir)
            .iter()
            .filter_map(|file| read_or_set_aside::<Book>(file))
            .filter(|book| {
                accounts
                    .iter()
                    .any(|account| account.id == book.info.account)
            })
            .collect();
        for book in &mut books {
            for stored in &mut book.cards {
                stored.card = parse_one(&stored.data);
            }
        }
        books.sort_by(|a, b| a.info.name.cmp(&b.info.name));
        let mut library = Self {
            dir: Some(dir),
            accounts,
            books,
        };
        library.ensure_local();
        library
    }

    fn ensure_local(&mut self) {
        if !self
            .accounts
            .iter()
            .any(|account| account.kind == AccountKind::Local)
        {
            self.accounts.insert(
                0,
                Account {
                    id: LOCAL_ACCOUNT.to_owned(),
                    name: String::new(),
                    kind: AccountKind::Local,
                },
            );
            self.save_accounts();
        }
        if !self
            .books
            .iter()
            .any(|book| book.info.account == LOCAL_ACCOUNT)
        {
            self.add_book(BookInfo {
                id: new_uid(),
                account: LOCAL_ACCOUNT.to_owned(),
                url: None,
                name: String::new(),
                read_only: false,
                ctag: None,
                sync_token: None,
            });
        }
    }

    pub fn save_accounts(&self) {
        let Some(dir) = &self.dir else {
            return;
        };
        let written = serde_json::to_string_pretty(&self.accounts)
            .map_err(std::io::Error::other)
            .and_then(|text| bande_xdg::replace_private_file(&dir.join(ACCOUNTS_FILE), &text));
        if let Err(error) = written {
            tracing::warn!("cannot save the accounts: {error}");
        }
    }

    pub fn save(&self, index: usize) {
        let (Some(dir), Some(book)) = (&self.dir, self.books.get(index)) else {
            return;
        };
        let file = book_file(dir, &book.info.id);
        let written = serde_json::to_string(book)
            .map_err(std::io::Error::other)
            .and_then(|text| bande_xdg::replace_private_file(&file, &text));
        if let Err(error) = written {
            tracing::warn!("cannot save {}: {error}", file.display());
        }
    }

    pub fn add_book(&mut self, info: BookInfo) -> usize {
        self.books.push(Book::new(info));
        let index = self.books.len() - 1;
        self.save(index);
        index
    }

    pub fn add_account(&mut self, account: Account, books: Vec<BookInfo>) {
        self.accounts.push(account);
        self.save_accounts();
        for info in books {
            self.add_book(info);
        }
    }

    pub fn update_account(&mut self, id: &str, kind: AccountKind, books: Vec<BookInfo>) {
        let Some(account) = self.accounts.iter_mut().find(|account| account.id == id) else {
            return;
        };
        account.kind = kind;
        self.save_accounts();
        while let Some(index) = self.books.iter().position(|book| {
            book.info.account == id
                && !books
                    .iter()
                    .any(|found| found.url.is_some() && found.url == book.info.url)
        }) {
            self.remove_book(index);
        }
        for info in books {
            if !self
                .books
                .iter()
                .any(|book| book.info.account == id && book.info.url == info.url)
            {
                self.add_book(info);
            }
        }
    }

    fn remove_book(&mut self, index: usize) {
        if index >= self.books.len() {
            return;
        }
        let book = self.books.remove(index);
        if let Some(dir) = &self.dir
            && let Err(error) = std::fs::remove_file(book_file(dir, &book.info.id))
        {
            tracing::warn!("cannot remove {}: {error}", book.info.name);
        }
    }

    pub fn remove_account(&mut self, id: &str) {
        if id == LOCAL_ACCOUNT {
            return;
        }
        while let Some(index) = self.books.iter().position(|book| book.info.account == id) {
            self.remove_book(index);
        }
        self.accounts.retain(|account| account.id != id);
        self.save_accounts();
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| account.id == id)
    }

    pub fn book_index(&self, id: &str) -> Option<usize> {
        self.books.iter().position(|book| book.info.id == id)
    }

    pub fn card(&self, found: &Found) -> Option<&Card> {
        self.books.get(found.book)?.cards.get(found.card)?.shown()
    }

    /// Every contact that is shown, with its book and position.
    pub fn contacts(&self) -> Vec<(Found, &Card)> {
        self.books
            .iter()
            .enumerate()
            .flat_map(|(book_index, book)| {
                book.cards
                    .iter()
                    .enumerate()
                    .filter_map(move |(card_index, stored)| {
                        let card = stored.shown()?;
                        Some((
                            Found {
                                book: book_index,
                                card: card_index,
                            },
                            card,
                        ))
                    })
            })
            .collect()
    }

    fn local(&self, book: usize) -> bool {
        self.books
            .get(book)
            .is_some_and(|book| book.info.account == LOCAL_ACCOUNT)
    }

    /// Stores `card` in `book`, replacing the card with its UID, as a pending change for a
    /// CardDAV book. Returns where it is, or nothing for a read-only book.
    pub fn put_card(&mut self, book: usize, card: Card) -> Option<usize> {
        let info = self.books.get(book)?.info.clone();
        if info.read_only {
            return None;
        }
        let local = self.local(book);
        let data = card.to_vcf();
        let target = self.books.get_mut(book)?;
        let index = match target.find(&card.uid()) {
            Some(index) => {
                let stored = &mut target.cards[index];
                stored.pending = match (local, stored.pending) {
                    (true, _) => None,
                    (false, Some(Pending::Create)) => Some(Pending::Create),
                    (false, _) => Some(Pending::Update),
                };
                stored.data = data;
                stored.card = Some(card);
                index
            }
            None => {
                let href = href_for(info.url.as_deref(), &card.uid(), "vcf");
                let pending = (!local).then_some(Pending::Create);
                target.cards.push(Stored {
                    href,
                    etag: None,
                    data,
                    pending,
                    card: Some(card),
                });
                target.cards.len() - 1
            }
        };
        self.save(book);
        Some(index)
    }

    /// Changes the contact at `found`, keeping every property the edit leaves alone.
    pub fn edit(&mut self, found: &Found, contact: &Contact) -> Option<usize> {
        let mut card = self.card(found)?.clone();
        let before = card.contact();
        card.apply(&before, contact);
        self.put_card(found.book, card)
    }

    /// Removes the card at `found`; returns its href and data for Undo.
    pub fn remove_card(&mut self, found: &Found) -> Option<(String, String)> {
        let local = self.local(found.book);
        let target = self.books.get_mut(found.book)?;
        if target.info.read_only {
            return None;
        }
        let stored = target.cards.get_mut(found.card)?;
        let removed = (stored.href.clone(), stored.data.clone());
        match (local, stored.pending) {
            (true, _) | (false, Some(Pending::Create)) => {
                target.cards.remove(found.card);
            }
            (false, _) => stored.pending = Some(Pending::Delete),
        }
        self.save(found.book);
        Some(removed)
    }

    /// Puts a removed card back at its own address, so other clients see it return.
    pub fn restore(&mut self, book: usize, href: &str, data: String) -> Option<usize> {
        let local = self.local(book);
        let target = self.books.get_mut(book)?;
        if target.info.read_only {
            return None;
        }
        let index = match target.cards.iter().position(|stored| stored.href == href) {
            Some(index) => {
                let stored = &mut target.cards[index];
                stored.pending = match (local, stored.pending) {
                    (true, _) => None,
                    (false, Some(Pending::Delete)) => {
                        stored.etag = None;
                        Some(Pending::Create)
                    }
                    (false, Some(Pending::Create)) => Some(Pending::Create),
                    (false, _) => Some(Pending::Update),
                };
                stored.data = data;
                stored.card = parse_one(&stored.data);
                index
            }
            None => {
                let pending = (!local).then_some(Pending::Create);
                target
                    .cards
                    .push(Stored::new(href.to_owned(), None, data, pending));
                target.cards.len() - 1
            }
        };
        self.save(book);
        Some(index)
    }
}
