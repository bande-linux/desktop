#![allow(clippy::unwrap_used, clippy::panic)]

use bande_calendar::caldav::{Availability, availability, scheduling, search_principals};
use bande_calendar::invite::UserType;
use bande_calendar::schedule::{Hours, free_slots};
use bande_dav::{Client, Precondition, Service};
use bande_test_bus::xandikos::Xandikos;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

#[test]
fn find_a_time_skips_what_the_server_says_is_busy() {
    let Some(server) = Xandikos::start().unwrap() else {
        return;
    };
    let client = Client::new(Service::CalDav, None);
    let found = client.discover_calendars(server.url()).unwrap();
    assert_eq!(found.addresses, ["jane@bande.test"]);
    let calendar = &found.calendars[0];
    assert!(!calendar.read_only);
    let event = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:test\r\nBEGIN:VEVENT\r\nUID:busy\r\n\
DTSTAMP:20261101T000000Z\r\nDTSTART:20261124T100000Z\r\nDTEND:20261124T110000Z\r\n\
SUMMARY:Review\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let url = format!("{}busy.ics", calendar.url);
    client.put(&url, event, &Precondition::Create).unwrap();

    let scheduling = scheduling(&client, &found.home).unwrap();
    let people = search_principals(&client, &scheduling.principal, "bande.test").unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].addresses, ["jane@bande.test"]);
    assert_eq!(people[0].kind, UserType::Individual);

    let day = (at("2026-11-24T00:00:00Z"), at("2026-11-25T00:00:00Z"));
    let answers = availability(
        &client,
        &scheduling,
        "jane@bande.test",
        &found.addresses,
        day,
    )
    .unwrap();
    let busy = vec![(at("2026-11-24T10:00:00Z"), at("2026-11-24T11:00:00Z"))];
    assert_eq!(
        answers,
        [(
            "jane@bande.test".to_owned(),
            Availability::Busy(busy.clone())
        )]
    );

    let hours = Hours {
        zone: &TimeZone::UTC,
        day: (9, 17),
        weekend: &|_| false,
    };
    let starts = free_slots(
        &busy,
        SignedDuration::from_hours(1),
        at("2026-11-24T08:50:00Z"),
        &hours,
        1,
        3,
    );
    assert_eq!(
        starts,
        ["09:00", "11:00", "12:00"].map(|time| at(&format!("2026-11-24T{time}:00Z")))
    );
}
