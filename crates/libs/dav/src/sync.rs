use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::client::{Client, Precondition, Resource};
use crate::http::{Error, resource_key};

/// A local change that the server does not have yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// The local change that was sent for a resource, and whether the server took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub pending: Pending,
    pub data: String,
    pub taken: bool,
}

impl Sent {
    fn still<T: Cached>(&self, item: &T) -> bool {
        item.pending() == Some(self.pending) && item.data() == self.data
    }
}

/// A resource as the server has it now; `sent` is the local change that was sent, if any, so
/// that one made while syncing is not overwritten.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upsert {
    pub href: String,
    pub etag: Option<String>,
    pub data: String,
    pub sent: Option<Sent>,
}

/// A local change that the server refused because the resource changed there first
/// (`412 Precondition Failed`); the server's version is in the outcome's upserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub href: String,
    pub data: String,
    pub pending: Pending,
}

/// A local change that the server answered with `status` and did not take. It stays pending
/// and is sent again at the next sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub href: String,
    pub data: String,
    pub status: u16,
}

/// What a sync did. With an `error` it stopped early: what it did until then still holds, and
/// the tags are those from before, so the next sync finds the rest.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub upserts: Vec<Upsert>,
    pub removed: Vec<String>,
    pub conflicts: Vec<Conflict>,
    pub refused: Vec<Refused>,
    pub error: Option<Error>,
}

fn conflict(
    client: &Client,
    local: &Local<'_>,
    pending: Pending,
    outcome: &mut Outcome,
) -> Result<(), Error> {
    match client.get(local.href) {
        Ok(Resource { etag, data, .. }) => outcome.upserts.push(Upsert {
            href: local.href.to_owned(),
            etag,
            data,
            sent: Some(Sent {
                pending,
                data: local.data.to_owned(),
                taken: false,
            }),
        }),
        Err(Error::Status { status: 404, .. }) if pending == Pending::Create => return Ok(()),
        Err(Error::Status { status: 404, .. }) => outcome.removed.push(local.href.to_owned()),
        Err(error) => return Err(error),
    }
    outcome.conflicts.push(Conflict {
        href: local.href.to_owned(),
        data: local.data.to_owned(),
        pending,
    });
    Ok(())
}

fn refuses_one_resource(status: u16) -> bool {
    !matches!(status, 408 | 429 | 502..=504) && status >= 400
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
                sent: Some(Sent {
                    pending,
                    data: local.data.to_owned(),
                    taken: true,
                }),
            }),
            Ok(None) => outcome.removed.push(local.href.to_owned()),
            Err(Error::PreconditionFailed) => conflict(client, local, pending, outcome)?,
            Err(Error::Status { status, .. }) if refuses_one_resource(status) => {
                outcome.refused.push(Refused {
                    href: local.href.to_owned(),
                    data: local.data.to_owned(),
                    status,
                });
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

struct Known {
    href: String,
    etag: Option<String>,
}

fn known_resources(items: &[Local<'_>], outcome: &Outcome) -> HashMap<String, Known> {
    let mut known = HashMap::new();
    let mut know = |href: &str, etag: Option<&str>| {
        known.insert(
            resource_key(href),
            Known {
                href: href.to_owned(),
                etag: etag.map(str::to_owned),
            },
        );
    };
    for local in items {
        if local.pending != Some(Pending::Create) {
            know(local.href, local.etag);
        }
    }
    for upsert in &outcome.upserts {
        know(&upsert.href, upsert.etag.as_deref());
    }
    for removed in &outcome.removed {
        known.remove(&resource_key(removed));
    }
    known
}

fn fetch_changed(
    client: &Client,
    url: &str,
    listed: Vec<(String, Option<String>)>,
    known: &HashMap<String, Known>,
    outcome: &mut Outcome,
) -> Result<(), Error> {
    let wanted: Vec<String> = listed
        .into_iter()
        .filter(|(href, etag)| {
            let mine = known.get(&resource_key(href)).map(|known| &known.etag);
            etag.is_none() || mine != Some(etag)
        })
        .map(|(href, _)| href)
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let mut upserted: HashMap<String, usize> = outcome
        .upserts
        .iter()
        .enumerate()
        .map(|(index, upsert)| (upsert.href.clone(), index))
        .collect();
    for resource in client.multiget(url, &wanted)? {
        let href = known
            .get(&resource_key(&resource.url))
            .map_or(resource.url, |known| known.href.clone());
        let at = *upserted
            .entry(href.clone())
            .or_insert(outcome.upserts.len());
        match outcome.upserts.get_mut(at) {
            Some(upsert) => {
                upsert.etag = resource.etag;
                upsert.data = resource.data;
            }
            None => outcome.upserts.push(Upsert {
                href,
                etag: resource.etag,
                data: resource.data,
                sent: None,
            }),
        }
    }
    Ok(())
}

fn forget_unlisted<'a>(
    listed: impl Iterator<Item = &'a String>,
    known: &HashMap<String, Known>,
    outcome: &mut Outcome,
) {
    let listed: HashSet<String> = listed.map(|href| resource_key(href)).collect();
    outcome.removed.extend(
        known
            .iter()
            .filter(|(key, _)| !listed.contains(*key))
            .map(|(_, known)| known.href.clone()),
    );
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
) -> Outcome {
    let mut outcome = Outcome {
        ctag: ctag.map(str::to_owned),
        sync_token: sync_token.map(str::to_owned),
        ..Outcome::default()
    };
    outcome.error = push(client, items, &mut outcome)
        .and_then(|()| pull(client, url, items, &mut outcome))
        .err();
    let refused: Vec<&String> = outcome
        .refused
        .iter()
        .map(|refused| &refused.href)
        .collect();
    outcome.removed.retain(|href| !refused.contains(&href));
    outcome
}

fn pull(
    client: &Client,
    url: &str,
    items: &[Local<'_>],
    outcome: &mut Outcome,
) -> Result<(), Error> {
    let known = known_resources(items, outcome);
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
            forget_unlisted(delta.changed.iter().map(|(href, _)| href), &known, outcome);
        }
        for href in &delta.removed {
            if let Some(mine) = known.get(&resource_key(href)) {
                outcome.removed.push(mine.href.clone());
            }
        }
        fetch_changed(client, url, delta.changed, &known, outcome)?;
        outcome.sync_token = delta.token.or(token);
    } else if token.is_none() && (ctag.is_none() || ctag != outcome.ctag || has_pending) {
        let listed = client.etags(url)?;
        forget_unlisted(listed.iter().map(|(href, _)| href), &known, outcome);
        fetch_changed(client, url, listed, &known, outcome)?;
    }
    outcome.ctag = ctag;
    Ok(())
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
/// version of what changed, and adds `copies` (local edits kept beside a conflict) as new
/// resources to push. What was edited or deleted while the sync ran stays a pending change.
pub fn merge<T: Cached>(
    items: &mut Vec<T>,
    removed: &[String],
    upserts: Vec<Upsert>,
    copies: Vec<Upsert>,
) {
    let kept = |item: &T| {
        copies
            .iter()
            .any(|copy| copy.sent.as_ref().is_some_and(|sent| sent.still(item)))
    };
    let removed: HashSet<&str> = removed.iter().map(String::as_str).collect();
    items.retain_mut(|item| {
        if !removed.contains(item.href()) {
            return true;
        }
        match item.pending() {
            Some(Pending::Create) => true,
            Some(Pending::Update) if !kept(item) => {
                item.set_etag(None);
                item.set_pending(Some(Pending::Create));
                true
            }
            _ => false,
        }
    });
    let mut places: HashMap<String, usize> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        places.entry(item.href().to_owned()).or_insert(index);
    }
    for upsert in upserts {
        let taken = upsert.sent.as_ref().is_some_and(|sent| sent.taken);
        let place = *places.entry(upsert.href.clone()).or_insert(items.len());
        match items.get_mut(place) {
            Some(item) => {
                let changed_meanwhile = item.pending().is_some()
                    && !upsert.sent.as_ref().is_some_and(|sent| sent.still(item));
                if !changed_meanwhile {
                    *item = T::fresh(upsert.href, upsert.etag, upsert.data, None);
                } else if taken {
                    item.set_etag(upsert.etag);
                    if item.pending() == Some(Pending::Create) {
                        item.set_pending(Some(Pending::Update));
                    }
                }
            }
            None => {
                let pending = taken.then_some(Pending::Delete);
                items.push(T::fresh(upsert.href, upsert.etag, upsert.data, pending));
            }
        }
    }
    items.extend(
        copies
            .into_iter()
            .map(|copy| T::fresh(copy.href, None, copy.data, Some(Pending::Create))),
    );
}
