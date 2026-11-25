use bande_dav::{
    Applied, Client, Conflict, Error, KeptCopy, Local, Sent, Upsert, merge, sync_collection,
};
use jiff::Timestamp;

use crate::ical::CalendarObject;
use crate::store::{Calendar, CalendarInfo, Library, Pending, href_for, new_uid};

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub calendar: String,
    pub ctag: Option<String>,
    pub sync_token: Option<String>,
    pub upserts: Vec<Upsert>,
    pub removed: Vec<String>,
    pub conflicts: Vec<String>,
    pub copies: Vec<KeptCopy>,
    pub refused: Vec<(String, u16)>,
    pub error: Option<Error>,
}

fn summary(data: &str) -> String {
    CalendarObject::parse(data)
        .ok()
        .and_then(|objects| objects.into_iter().next())
        .and_then(|object| {
            object
                .master()
                .or(object.events.first())
                .map(|event| event.summary.clone())
        })
        .unwrap_or_default()
}

/// The event of `data` as one of its own with `uid`, without the people invited.
pub fn copied(data: &str, uid: &str) -> Option<String> {
    let mut object = CalendarObject::parse(data).ok()?.into_iter().next()?;
    for event in &mut object.events {
        uid.clone_into(&mut event.uid);
        event.component.properties.retain(|property| {
            !["ORGANIZER", "ATTENDEE"]
                .iter()
                .any(|name| property.name.eq_ignore_ascii_case(name))
        });
    }
    Some(object.to_ics(Timestamp::now()))
}

fn kept_copy(info: &CalendarInfo, conflict: &Conflict) -> Option<KeptCopy> {
    let uid = new_uid();
    let data = copied(&conflict.data, &uid)?;
    Some(KeptCopy {
        href: href_for(info, &uid),
        replaces: conflict.href.clone(),
        data,
        sent: Sent {
            pending: conflict.pending,
            data: conflict.data.clone(),
            taken: false,
        },
    })
}

pub fn sync_calendar(client: &Client, calendar: &Calendar) -> Result<Outcome, Error> {
    let url = calendar
        .info
        .url
        .clone()
        .ok_or_else(|| Error::NotFound(calendar.info.name.clone()))?;
    let items: Vec<Local<'_>> = calendar
        .objects
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
        calendar.info.ctag.as_deref(),
        calendar.info.sync_token.as_deref(),
        &items,
    );
    Ok(Outcome {
        calendar: calendar.info.id.clone(),
        ctag: synced.ctag,
        sync_token: synced.sync_token,
        upserts: synced.upserts,
        removed: synced.removed,
        conflicts: synced
            .conflicts
            .iter()
            .map(|conflict| summary(&conflict.data))
            .collect(),
        copies: synced
            .conflicts
            .iter()
            .filter(|conflict| conflict.pending != Pending::Delete)
            .filter_map(|conflict| kept_copy(&calendar.info, conflict))
            .collect(),
        refused: synced
            .refused
            .iter()
            .map(|refused| (summary(&refused.data), refused.status))
            .collect(),
        error: synced.error,
    })
}

pub fn subscription_outcome(info: &CalendarInfo, data: &str, alerts: bool) -> Option<Outcome> {
    let mut objects = CalendarObject::parse(data).ok()?;
    if !alerts {
        for event in objects
            .iter_mut()
            .flat_map(|object| object.events.iter_mut())
        {
            event.alarms.clear();
        }
    }
    let stamp = Timestamp::now();
    Some(Outcome {
        calendar: info.id.clone(),
        ctag: None,
        sync_token: None,
        upserts: objects
            .iter()
            .map(|object| Upsert {
                href: format!("{}.ics", object.uid()),
                etag: None,
                data: object.to_ics(stamp),
                sent: None,
            })
            .collect(),
        removed: Vec::new(),
        conflicts: Vec::new(),
        copies: Vec::new(),
        refused: Vec::new(),
        error: None,
    })
}

pub fn apply(library: &mut Library, outcome: Outcome, replace_all: bool) -> Applied {
    let Some(index) = library.calendar_index(&outcome.calendar) else {
        return Applied::default();
    };
    let calendar = &mut library.calendars[index];
    calendar.info.ctag = outcome.ctag;
    calendar.info.sync_token = outcome.sync_token;
    if replace_all {
        calendar.objects.clear();
    }
    let moved = merge(
        &mut calendar.objects,
        &outcome.removed,
        outcome.upserts,
        outcome.copies,
    );
    library.save(index);
    Applied {
        conflicts: outcome.conflicts,
        moved,
    }
}
