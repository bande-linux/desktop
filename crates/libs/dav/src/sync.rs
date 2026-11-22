use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::client::{Client, Precondition, Resource};
use crate::http::{Error, same_resource};

/// A local change that the server does not have yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pending {
    Create,
    Update,
    Delete,
}

/// One resource of the offline copy of a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local<'a> {
    pub href: &'a str,
    pub etag: Option<&'a str>,
    pub data: &'a str,
    pub pending: Option<Pending>,
}

/// A resource as the server has it now; `pushed` is the local data that was sent, if any, so
/// that an edit made while syncing is not overwritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upsert {
    pub href: String,
    pub etag: Option<String>,
    pub data: String,
    pub pushed: Option<String>,
}

/// A local change that the server refused because the resource changed there first
/// (`412 Precondition Failed`); the server's version is in the outcome's upserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub href: String,
    pub data: String,
    pub pending: Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub upserts: Vec<Upsert>,
    pub removed: Vec<String>,
    pub conflicts: Vec<Conflict>,
}

fn conflict(client: &Client, local: &Local<'_>, outcome: &mut Outcome) -> Result<(), Error> {
    if let Some(pending) = local.pending {
        outcome.conflicts.push(Conflict {
            href: local.href.to_owned(),
            data: local.data.to_owned(),
            pending,
        });
    }
    match client.get(local.href) {
        Ok(Resource { etag, data, .. }) => outcome.upserts.push(Upsert {
            href: local.href.to_owned(),
            etag,
            data,
            pushed: Some(local.data.to_owned()),
        }),
        Err(Error::Status { status: 404, .. }) => outcome.removed.push(local.href.to_owned()),
        Err(error) => return Err(error),
    }
    Ok(())
}

fn push(client: &Client, items: &[Local<'_>], outcome: &mut Outcome) -> Result<(), Error> {
    for local in items {
        let Some(pending) = local.pending else {
            continue;
        };
        let result = match pending {
            Pending::Create => match client.put(local.href, local.data, &Precondition::Create) {
                Err(Error::PreconditionFailed) => client.get(local.href).and_then(|existing| {
                    let precondition = existing
                        .etag
                        .map_or(Precondition::None, Precondition::Match);
                    client.put(local.href, local.data, &precondition)
                }),
                other => other,
            }
            .map(Some),
            Pending::Update => {
                let precondition = local.etag.map_or(Precondition::None, |etag| {
                    Precondition::Match(etag.to_owned())
                });
                client.put(local.href, local.data, &precondition).map(Some)
            }
            Pending::Delete => client.delete(local.href, local.etag).map(|()| None),
        };
        match result {
            Ok(Some(etag)) => outcome.upserts.push(Upsert {
                href: local.href.to_owned(),
                etag,
                data: local.data.to_owned(),
                pushed: Some(local.data.to_owned()),
            }),
            Ok(None) => outcome.removed.push(local.href.to_owned()),
            Err(Error::PreconditionFailed) => conflict(client, local, outcome)?,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn known_etags(items: &[Local<'_>], outcome: &Outcome) -> HashMap<String, Option<String>> {
    let mut known: HashMap<String, Option<String>> = items
        .iter()
        .filter(|local| local.pending != Some(Pending::Create))
        .map(|local| (local.href.to_owned(), local.etag.map(str::to_owned)))
        .collect();
    for upsert in &outcome.upserts {
        known.insert(upsert.href.clone(), upsert.etag.clone());
    }
    for removed in &outcome.removed {
        known.remove(removed);
    }
    known
}

fn fetch_changed(
    client: &Client,
    url: &str,
    listed: Vec<(String, Option<String>)>,
    known: &HashMap<String, Option<String>>,
    outcome: &mut Outcome,
) -> Result<(), Error> {
    let wanted: Vec<String> = listed
        .into_iter()
        .filter(|(href, etag)| {
            let mine = known
                .iter()
                .find(|(known, _)| same_resource(known, href))
                .map(|(_, etag)| etag);
            etag.is_none() || mine != Some(etag)
        })
        .map(|(href, _)| href)
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    for resource in client.multiget(url, &wanted)? {
        let href = known
            .keys()
            .find(|known| same_resource(known, &resource.url))
            .cloned()
            .unwrap_or(resource.url);
        match outcome
            .upserts
            .iter_mut()
            .find(|upsert| upsert.href == href)
        {
            Some(upsert) => {
                upsert.etag = resource.etag;
                upsert.data = resource.data;
            }
            None => outcome.upserts.push(Upsert {
                href,
                etag: resource.etag,
                data: resource.data,
                pushed: None,
            }),
        }
    }
    Ok(())
}

fn forget_unlisted<'a>(
    listed: impl Iterator<Item = &'a String> + Clone,
    known: &HashMap<String, Option<String>>,
    outcome: &mut Outcome,
) {
    for href in known.keys() {
        if !listed.clone().any(|listed| same_resource(listed, href))
            && !outcome.removed.contains(href)
        {
            outcome.removed.push(href.clone());
        }
    }
}

/// Pushes the pending changes of the collection at `url`, then pulls what changed on the
/// server: with `sync-collection` (RFC 6578) when the server offers a sync token, else by
/// comparing etags when its ctag changed.
pub fn sync_collection(
    client: &Client,
    url: &str,
    ctag: Option<&str>,
    sync_token: Option<&str>,
    items: &[Local<'_>],
) -> Result<Outcome, Error> {
    let mut outcome = Outcome {
        ctag: ctag.map(str::to_owned),
        sync_token: sync_token.map(str::to_owned),
        ..Outcome::default()
    };
    push(client, items, &mut outcome)?;
    let known = known_etags(items, &outcome);
    let (ctag, token) = client.collection_tags(url)?;
    let has_pending = items.iter().any(|local| local.pending.is_some());
    if token.is_some() && token != outcome.sync_token {
        let delta = match client.sync_collection(url, outcome.sync_token.as_deref()) {
            Ok(delta) => Some(delta),
            Err(Error::Status {
                status: 403 | 409 | 400,
                ..
            }) if outcome.sync_token.is_some() => None,
            Err(error) => return Err(error),
        };
        let (delta, full) = match delta {
            Some(delta) => (delta, outcome.sync_token.is_none()),
            None => (client.sync_collection(url, None)?, true),
        };
        if full {
            forget_unlisted(
                delta.changed.iter().map(|(href, _)| href),
                &known,
                &mut outcome,
            );
        }
        for href in &delta.removed {
            if let Some(mine) = known.keys().find(|known| same_resource(known, href)) {
                outcome.removed.push(mine.clone());
            }
        }
        fetch_changed(client, url, delta.changed, &known, &mut outcome)?;
        outcome.sync_token = delta.token.or(token);
    } else if token.is_none() && (ctag.is_none() || ctag != outcome.ctag || has_pending) {
        let listed = client.etags(url)?;
        forget_unlisted(listed.iter().map(|(href, _)| href), &known, &mut outcome);
        fetch_changed(client, url, listed, &known, &mut outcome)?;
    }
    outcome.ctag = ctag;
    Ok(outcome)
}

/// A resource of an app's offline copy, which [`merge`] updates from a sync's outcome.
pub trait Cached: Sized {
    fn fresh(href: String, etag: Option<String>, data: String, pending: Option<Pending>) -> Self;
    fn href(&self) -> &str;
    fn data(&self) -> &str;
    fn pending(&self) -> Option<Pending>;
    fn set_etag(&mut self, etag: Option<String>);
    fn set_pending(&mut self, pending: Option<Pending>);
}

/// Applies what a sync found to `items`: removes what the server removed, takes the server's
/// version of what changed unless it was edited again while the sync ran, and adds `copies`
/// (local edits kept beside a conflict) as new resources to push.
pub fn merge<T: Cached>(
    items: &mut Vec<T>,
    removed: &[String],
    upserts: Vec<Upsert>,
    copies: Vec<Upsert>,
) {
    items.retain(|item| {
        item.pending() == Some(Pending::Create) || !removed.iter().any(|href| href == item.href())
    });
    for upsert in upserts {
        match items.iter_mut().find(|item| item.href() == upsert.href) {
            Some(item) => {
                let edited_meanwhile =
                    item.pending().is_some() && upsert.pushed.as_deref() != Some(item.data());
                if !edited_meanwhile {
                    *item = T::fresh(upsert.href, upsert.etag, upsert.data, None);
                } else {
                    item.set_etag(upsert.etag);
                    if item.pending() == Some(Pending::Create) {
                        item.set_pending(Some(Pending::Update));
                    }
                }
            }
            None => items.push(T::fresh(upsert.href, upsert.etag, upsert.data, None)),
        }
    }
    items.extend(
        copies
            .into_iter()
            .map(|copy| T::fresh(copy.href, None, copy.data, Some(Pending::Create))),
    );
}
