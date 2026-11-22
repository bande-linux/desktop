use crate::client::{Client, last_segment};
use crate::http::{Error, join};
use crate::xml::{self, CALSERVER, CARDDAV, DAV};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAddressBook {
    pub url: String,
    pub name: String,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundAddressBooks {
    pub principal: String,
    pub home: String,
    pub books: Vec<RemoteAddressBook>,
}

impl Client {
    /// Signs in at `server` and follows `current-user-principal` → `addressbook-home-set`
    /// (RFC 6352) → the address books there.
    pub fn discover_address_books(&self, server: &str) -> Result<FoundAddressBooks, Error> {
        self.discover_with(server, |url| {
            let (principal, home) = self.principal_and_home(
                url,
                (CARDDAV, "addressbook-home-set", "card:addressbook-home-set"),
            )?;
            let books = self.address_books(&home)?;
            match books.is_empty() {
                true => Err(Error::NotFound(home)),
                false => Ok(FoundAddressBooks {
                    principal,
                    home,
                    books,
                }),
            }
        })
    }

    pub fn address_books(&self, home: &str) -> Result<Vec<RemoteAddressBook>, Error> {
        let body = xml::propfind(&[
            "d:resourcetype",
            "d:displayname",
            "cs:getctag",
            "d:sync-token",
            "d:current-user-privilege-set",
        ]);
        let (url, status) = self.multistatus("PROPFIND", home, "1", &body)?;
        Ok(status
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .prop(DAV, "resourcetype")
                    .is_some_and(|kind| kind.child(CARDDAV, "addressbook").is_some())
            })
            .map(|entry| {
                let book_url = join(&url, &entry.href);
                RemoteAddressBook {
                    name: entry
                        .text(DAV, "displayname")
                        .map(str::to_owned)
                        .unwrap_or_else(|| last_segment(&book_url)),
                    ctag: entry.text(CALSERVER, "getctag").map(str::to_owned),
                    sync_token: entry.text(DAV, "sync-token").map(str::to_owned),
                    read_only: entry.read_only(),
                    url: book_url,
                }
            })
            .collect())
    }
}
