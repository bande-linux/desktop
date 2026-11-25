use jiff::Timestamp;

use crate::ical::freebusy::{self, Period};
use crate::invite::UserType;
use bande_dav::xml::{CALDAV, DAV, Element, escape, parse_element};
use bande_dav::{Client, Error, join, mail_address};

const SEARCH_LIMIT: usize = 20;
const NAMESPACES: &str = "xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub url: String,
    pub name: String,
    pub addresses: Vec<String>,
    pub kind: UserType,
    pub home: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scheduling {
    pub principal: String,
    pub outbox: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    Busy(Vec<Period>),
    Unknown,
}

fn utc(at: Timestamp) -> String {
    at.strftime("%Y%m%dT%H%M%SZ").to_string()
}

fn principal_search(prop: &str, text: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<d:principal-property-search {NAMESPACES}><d:property-search><d:prop><{prop}/></d:prop><d:match>{}</d:match></d:property-search><d:prop><d:displayname/><c:calendar-user-address-set/><c:calendar-user-type/><c:calendar-home-set/></d:prop><d:apply-to-principal-collection-set/></d:principal-property-search>",
        escape(text)
    )
}

fn free_busy_query((from, to): Period) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<c:free-busy-query {NAMESPACES}><c:time-range start=\"{}\" end=\"{}\"/></c:free-busy-query>",
        utc(from),
        utc(to)
    )
}

fn answered(response: &Element) -> Option<(String, Availability)> {
    let recipient = response
        .child(CALDAV, "recipient")?
        .hrefs()
        .into_iter()
        .find_map(|href| mail_address(&href))?;
    let success = response
        .child(CALDAV, "request-status")
        .is_some_and(|status| status.text.trim_start().starts_with("2."));
    let busy = response
        .child(CALDAV, "calendar-data")
        .filter(|_| success)
        .and_then(|data| freebusy::busy(&data.text));
    Some((
        recipient,
        busy.map_or(Availability::Unknown, Availability::Busy),
    ))
}

fn tolerated(error: Error) -> Result<(), Error> {
    match error {
        Error::Status { status, url } => {
            tracing::debug!("{url} answered {status} to a scheduling request");
            Ok(())
        }
        Error::Xml(error) => {
            tracing::debug!("a scheduling answer is not valid: {error}");
            Ok(())
        }
        error => Err(error),
    }
}

/// The signed-in user's principal for the calendar home `home`, with its RFC 6638
/// scheduling outbox when the server has one.
pub fn scheduling(client: &Client, home: &str) -> Result<Scheduling, Error> {
    let principal = client
        .first_href(
            home,
            DAV,
            "current-user-principal",
            "d:current-user-principal",
        )?
        .map(|(_, principal)| principal)
        .ok_or_else(|| Error::NotFound(home.to_owned()))?;
    let outbox = match client.first_href(
        &principal,
        CALDAV,
        "schedule-outbox-URL",
        "c:schedule-outbox-URL",
    ) {
        Ok(outbox) => outbox.map(|(_, outbox)| outbox),
        Err(error) => {
            tolerated(error)?;
            None
        }
    };
    Ok(Scheduling { principal, outbox })
}

/// Principals whose name or calendar user address contains `text` (RFC 3744
/// principal-property-search over the server's principal collections).
pub fn search_principals(
    client: &Client,
    principal: &str,
    text: &str,
) -> Result<Vec<Principal>, Error> {
    let mut found: Vec<Principal> = Vec::new();
    for prop in ["d:displayname", "c:calendar-user-address-set"] {
        let (url, status) =
            match client.multistatus("REPORT", principal, "0", &principal_search(prop, text)) {
                Ok(answer) => answer,
                Err(error) => {
                    tolerated(error)?;
                    continue;
                }
            };
        for entry in status.entries {
            let url = join(&url, &entry.href);
            if found.iter().any(|known| known.url == url) {
                continue;
            }
            let addresses: Vec<String> = entry
                .prop(CALDAV, "calendar-user-address-set")
                .map(Element::hrefs)
                .unwrap_or_default()
                .iter()
                .filter_map(|href| mail_address(href))
                .collect();
            if addresses.is_empty() {
                continue;
            }
            found.push(Principal {
                name: entry
                    .text(DAV, "displayname")
                    .map_or_else(|| addresses[0].clone(), str::to_owned),
                kind: UserType::of(entry.text(CALDAV, "calendar-user-type")),
                home: entry
                    .prop(CALDAV, "calendar-home-set")
                    .and_then(|home| home.hrefs().into_iter().next())
                    .map(|home| join(&url, &home)),
                addresses,
                url,
            });
        }
    }
    found.truncate(SEARCH_LIMIT);
    Ok(found)
}

/// When each of `attendees` is busy during `window`: asked of the scheduling outbox
/// (RFC 6638 §6), and for anyone it does not answer for, with a free-busy-query
/// (RFC 4791 §7.10) on the calendars of their principal on this server.
pub fn availability(
    client: &Client,
    scheduling: &Scheduling,
    organizer: &str,
    attendees: &[String],
    window: Period,
) -> Result<Vec<(String, Availability)>, Error> {
    let mut found: Vec<(String, Availability)> = attendees
        .iter()
        .map(|attendee| (attendee.clone(), Availability::Unknown))
        .collect();
    if let Some(outbox) = &scheduling.outbox {
        match ask_outbox(client, outbox, organizer, attendees, window) {
            Ok(answers) => {
                for (recipient, availability) in answers {
                    if let Some((_, slot)) = found
                        .iter_mut()
                        .find(|(attendee, _)| attendee.eq_ignore_ascii_case(&recipient))
                    {
                        *slot = availability;
                    }
                }
            }
            Err(error) => tolerated(error)?,
        }
    }
    for (attendee, availability) in &mut found {
        if *availability == Availability::Unknown {
            *availability = query_principal(client, &scheduling.principal, attendee, window)?;
        }
    }
    Ok(found)
}

fn ask_outbox(
    client: &Client,
    outbox: &str,
    organizer: &str,
    attendees: &[String],
    window: Period,
) -> Result<Vec<(String, Availability)>, Error> {
    let body = freebusy::request(
        &crate::store::new_uid(),
        organizer,
        attendees,
        window,
        Timestamp::now(),
    );
    let response = client.send(
        "POST",
        outbox,
        &[("Content-Type", "text/calendar; charset=utf-8")],
        Some(&body),
    )?;
    if !response.success() {
        return Err(Error::Status {
            status: response.status,
            url: outbox.to_owned(),
        });
    }
    let root = parse_element(&response.body)?;
    Ok(root
        .children
        .iter()
        .filter(|child| child.is(CALDAV, "response"))
        .filter_map(answered)
        .collect())
}

fn query_principal(
    client: &Client,
    principal: &str,
    address: &str,
    window: Period,
) -> Result<Availability, Error> {
    let home = search_principals(client, principal, address)?
        .into_iter()
        .find(|found| {
            found
                .addresses
                .iter()
                .any(|known| known.eq_ignore_ascii_case(address))
        })
        .and_then(|found| found.home);
    let Some(home) = home else {
        return Ok(Availability::Unknown);
    };
    let calendars = match client.calendars(&home) {
        Ok(calendars) => calendars,
        Err(error) => {
            tolerated(error)?;
            return Ok(Availability::Unknown);
        }
    };
    let mut busy = Vec::new();
    let mut answered = false;
    for calendar in calendars {
        let response = client.send(
            "REPORT",
            &calendar.url,
            &[("Depth", "1")],
            Some(&free_busy_query(window)),
        )?;
        match response
            .success()
            .then(|| freebusy::busy(&response.body))
            .flatten()
        {
            Some(found) => {
                answered = true;
                busy.extend(found);
            }
            None => tracing::debug!(
                "{} answered {} to a free-busy-query",
                calendar.url,
                response.status
            ),
        }
    }
    busy.sort();
    Ok(match answered {
        true => Availability::Busy(busy),
        false => Availability::Unknown,
    })
}
