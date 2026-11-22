//! Contacts on open standards, shared by Contacts, Mail and Calendar: vCard 3.0 and 4.0, the
//! offline copy of every address book with its pending changes, CardDAV sync, and the
//! read-only [`Directory`] other apps complete addresses and show names from.

mod directory;
pub mod store;
pub mod sync;
pub mod vcard;

pub use directory::{Directory, Person, completes};
