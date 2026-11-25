use bande_dav::{Client, Error, Local, Upsert, merge, sync_collection};
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
    pub copies: Vec<Upsert>,
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

fn kept_copy(info: &CalendarInfo, data: &str) -> Option<Upsert> {
    let mut object = CalendarObject::parse(data).ok()?.into_iter().next()?;
    let uid = new_uid();
    for event in &mut object.events {
        event.uid = uid.clone();
        event.component.properties.retain(|property| {
            !["ORGANIZER", "ATTENDEE"]
                .iter()
                .any(|name| property.name.eq_ignore_ascii_case(name))
        });
    }
    Some(Upsert {
        href: href_for(info, &uid),
        etag: None,
        data: object.to_ics(Timestamp::now()),
        pushed: None,
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
    )?;
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
            .filter(|conflict| conflict.pending == Pending::Update)
            .filter_map(|conflict| kept_copy(&calendar.info, &conflict.data))
            .collect(),
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
                pushed: None,
            })
            .collect(),
        removed: Vec::new(),
        conflicts: Vec::new(),
        copies: Vec::new(),
    })
}

pub fn apply(library: &mut Library, outcome: Outcome, replace_all: bool) -> Vec<String> {
    let Some(index) = library.calendar_index(&outcome.calendar) else {
        return Vec::new();
    };
    let calendar = &mut library.calendars[index];
    calendar.info.ctag = outcome.ctag;
    calendar.info.sync_token = outcome.sync_token;
    if replace_all {
        calendar.objects.clear();
    }
    merge(
        &mut calendar.objects,
        &outcome.removed,
        outcome.upserts,
        outcome.copies,
    );
    library.save(index);
    outcome.conflicts
}
