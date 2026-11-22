use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use super::event::PRODUCT;
use super::lines::{self, Component, Property};
use super::tz::Zones;
use super::value::{Duration, Moment};

pub type Period = (Timestamp, Timestamp);

fn utc(at: Timestamp) -> String {
    at.strftime("%Y%m%dT%H%M%SZ").to_string()
}

pub fn request(
    uid: &str,
    organizer: &str,
    attendees: &[String],
    (from, to): Period,
    stamp: Timestamp,
) -> String {
    let mut calendar = Component::new("VCALENDAR");
    calendar.push(Property::new("VERSION", "2.0"));
    calendar.push(Property::new("PRODID", PRODUCT));
    calendar.push(Property::new("METHOD", "REQUEST"));
    let mut query = Component::new("VFREEBUSY");
    query.push(Property::new("UID", uid));
    query.push(Property::new("DTSTAMP", utc(stamp)));
    query.push(Property::new("DTSTART", utc(from)));
    query.push(Property::new("DTEND", utc(to)));
    query.push(Property::new("ORGANIZER", format!("mailto:{organizer}")));
    for attendee in attendees {
        query.push(Property::new("ATTENDEE", format!("mailto:{attendee}")));
    }
    calendar.components.push(query);
    calendar.to_ics()
}

fn period(text: &str, zones: &Zones) -> Option<Period> {
    let (start, end) = text.split_once('/')?;
    let start = Moment::parse_text(start.trim(), None, None, zones)?.timestamp(&TimeZone::UTC);
    let end = match end.trim() {
        length if length.starts_with(['P', 'p', '+']) => start
            .checked_add(SignedDuration::from_secs(
                Duration::parse(length)?.total_seconds(),
            ))
            .ok()?,
        end => Moment::parse_text(end, None, None, zones)?.timestamp(&TimeZone::UTC),
    };
    (end > start).then_some((start, end))
}

pub fn busy(text: &str) -> Option<Vec<Period>> {
    let roots = lines::parse(text).ok()?;
    let mut found = Vec::new();
    for root in roots.iter().filter(|root| root.name == "VCALENDAR") {
        let zones = Zones::from_calendar(root);
        for property in root
            .children("VFREEBUSY")
            .flat_map(|component| component.all("FREEBUSY"))
            .filter(|property| {
                property
                    .param("FBTYPE")
                    .is_none_or(|kind| !kind.eq_ignore_ascii_case("FREE"))
            })
        {
            found.extend(
                lines::split_list(&property.value)
                    .into_iter()
                    .filter_map(|text| period(text, &zones)),
            );
        }
    }
    found.sort();
    Some(found)
}
