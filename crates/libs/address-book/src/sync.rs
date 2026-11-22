use bande_dav::{Client, Error, Local, Pending, Upsert, href_for, merge, new_uid, sync_collection};

use crate::store::{Book, Library};
use crate::vcard::{display_name, parse_cards};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub book: String,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub upserts: Vec<Upsert>,
    pub removed: Vec<String>,
    pub conflicts: Vec<String>,
    pub copies: Vec<Upsert>,
}

fn name_of(data: &str) -> String {
    parse_cards(data)
        .cards
        .first()
        .map(|card| display_name(&card.contact()))
        .unwrap_or_default()
}

fn kept_copy(book: &Book, data: &str) -> Option<Upsert> {
    let mut card = parse_cards(data).cards.into_iter().next()?;
    let uid = new_uid();
    card.set_uid(&uid);
    Some(Upsert {
        href: href_for(book.info.url.as_deref(), &uid, "vcf"),
        etag: None,
        data: card.to_vcf(),
        pushed: None,
    })
}

/// Pushes the book's pending changes and pulls what changed on the server. An edit the
/// server refused because the contact changed there first is kept as a copy beside the
/// server's version, so nothing typed is lost and the two can be merged.
pub fn sync_book(client: &Client, book: &Book) -> Result<Outcome, Error> {
    let url = book
        .info
        .url
        .clone()
        .ok_or_else(|| Error::NotFound(book.info.name.clone()))?;
    let items: Vec<Local<'_>> = book
        .cards
        .iter()
        .map(|stored| Local {
            href: &stored.href,
            etag: stored.etag.as_deref(),
            data: &stored.data,
            pending: stored.pending,
        })
        .collect();
    let synced = sync_collection(
        client,
        &url,
        book.info.ctag.as_deref(),
        book.info.sync_token.as_deref(),
        &items,
    )?;
    Ok(Outcome {
        book: book.info.id.clone(),
        ctag: synced.ctag,
        sync_token: synced.sync_token,
        upserts: synced.upserts,
        removed: synced.removed,
        conflicts: synced
            .conflicts
            .iter()
            .map(|conflict| name_of(&conflict.data))
            .collect(),
        copies: synced
            .conflicts
            .iter()
            .filter(|conflict| conflict.pending == Pending::Update)
            .filter_map(|conflict| kept_copy(book, &conflict.data))
            .collect(),
    })
}

/// Applies a sync's outcome to the offline copy, keeping what was edited meanwhile; returns
/// the names of the contacts that met a conflict.
pub fn apply(library: &mut Library, outcome: Outcome) -> Vec<String> {
    let Some(index) = library.book_index(&outcome.book) else {
        return Vec::new();
    };
    let book = &mut library.books[index];
    book.info.ctag = outcome.ctag;
    book.info.sync_token = outcome.sync_token;
    merge(
        &mut book.cards,
        &outcome.removed,
        outcome.upserts,
        outcome.copies,
    );
    library.save(index);
    outcome.conflicts
}
