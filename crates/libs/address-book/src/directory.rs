use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::store::{Book, book_files, data_dir};
use crate::vcard::{Birthday, Kind, display_name};

/// A contact as other apps see it: Mail's address completion and sender names, Calendar's
/// invitee completion and birthdays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub uid: String,
    pub name: String,
    pub emails: Vec<String>,
    pub birthday: Option<Birthday>,
}

/// The contacts of every address book, read from Contacts' offline copy and read again when
/// it changes. It never touches the network.
#[derive(Debug, Default)]
pub struct Directory {
    dir: PathBuf,
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    people: Vec<Person>,
    names: HashMap<String, usize>,
}

fn stamps(dir: &Path) -> Vec<(PathBuf, Option<SystemTime>)> {
    book_files(dir)
        .into_iter()
        .map(|file| {
            let modified = std::fs::metadata(&file)
                .and_then(|metadata| metadata.modified())
                .ok();
            (file, modified)
        })
        .collect()
}

impl Directory {
    pub fn open() -> Self {
        Self::at(data_dir())
    }

    pub fn at(dir: PathBuf) -> Self {
        let mut directory = Self {
            dir,
            ..Self::default()
        };
        directory.refresh();
        directory
    }

    /// Reads the address books again if they changed; says whether they did.
    pub fn refresh(&mut self) -> bool {
        let now = stamps(&self.dir);
        if now == self.stamps {
            return false;
        }
        self.people = now
            .iter()
            .filter_map(|(file, _)| bande_dav::read_or_set_aside::<Book>(file))
            .flat_map(|book| book.cards)
            .filter(|stored| stored.pending != Some(bande_dav::Pending::Delete))
            .filter_map(|stored| {
                crate::vcard::parse_cards(&stored.data)
                    .cards
                    .into_iter()
                    .next()
            })
            .filter(|card| card.kind() == Kind::Individual)
            .map(|card| {
                let contact = card.contact();
                Person {
                    uid: card.uid(),
                    name: display_name(&contact),
                    emails: contact
                        .emails
                        .into_iter()
                        .map(|email| email.value)
                        .collect(),
                    birthday: contact.birthday,
                }
            })
            .collect();
        self.people
            .sort_by_cached_key(|person| person.name.to_lowercase());
        self.names = self
            .people
            .iter()
            .enumerate()
            .flat_map(|(index, person)| {
                person
                    .emails
                    .iter()
                    .map(move |email| (email.to_lowercase(), index))
            })
            .collect();
        self.stamps = now;
        true
    }

    pub fn people(&self) -> &[Person] {
        &self.people
    }

    /// Every `(name, address)` pair, by name.
    pub fn addresses(&self) -> impl Iterator<Item = (&str, &str)> {
        self.people.iter().flat_map(|person| {
            person
                .emails
                .iter()
                .map(|email| (person.name.as_str(), email.as_str()))
        })
    }

    /// The name of the contact with `email`, if there is one.
    pub fn name_for(&self, email: &str) -> Option<&str> {
        let index = *self.names.get(&email.trim().to_lowercase())?;
        self.people
            .get(index)
            .map(|person| person.name.as_str())
            .filter(|name| !name.is_empty())
    }

    pub fn knows(&self, email: &str) -> bool {
        self.names.contains_key(&email.trim().to_lowercase())
    }

    /// The rest of the first contact `token` starts, as `Name <address>` or the bare address.
    pub fn complete(&self, token: &str) -> Option<String> {
        self.addresses().find_map(|(name, email)| {
            let full = match name.is_empty() {
                true => email.to_owned(),
                false => format!("{name} <{email}>"),
            };
            completes(&full, token).or_else(|| completes(email, token))
        })
    }
}

/// The rest of `candidate` when it starts with `token`, ignoring case: what an address field
/// shows selected after the typed text.
pub fn completes(candidate: &str, token: &str) -> Option<String> {
    let mut rest = candidate.char_indices();
    for typed in token.chars() {
        let (_, known) = rest.next()?;
        if !known.to_lowercase().eq(typed.to_lowercase()) {
            return None;
        }
    }
    let rest = rest.as_str();
    (!rest.is_empty()).then(|| rest.to_owned())
}
