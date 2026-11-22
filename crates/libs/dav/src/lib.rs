//! WebDAV for CalDAV (RFC 4791) and CardDAV (RFC 6352): HTTP with Basic and Digest sign-in
//! over TLS only, multistatus XML, discovery (RFC 6764), offline sync with `sync-collection`
//! (RFC 6578) or etags, and account offers between the apps that speak them.

mod carddav;
mod client;
mod http;
pub mod offer;
mod store;
mod sync;
mod xml;

pub use carddav::{FoundAddressBooks, RemoteAddressBook};
pub use client::{
    Client, Delta, Discovered, Precondition, RemoteCalendar, Resource, Service, candidates,
};
pub use http::{Credentials, Error, join, same_resource};
pub use store::{href_for, new_uid, read_or_set_aside};
pub use sync::{Cached, Conflict, Local, Outcome, Pending, Upsert, merge, sync_collection};
