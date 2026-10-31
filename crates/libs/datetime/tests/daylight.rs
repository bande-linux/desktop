#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use jiff::civil::date;
use jiff::{Timestamp, Zoned};
use unbound_datetime::daylight::{Place, Sky, place, sky};

const ZONE_TAB: &str = "# comment\nSE\t+5920+01803\tEurope/Stockholm\nNO\t+5955+01045\tEurope/Oslo\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\n";

fn at(text: &str) -> Zoned {
    text.parse().expect("zoned")
}

fn local(timestamp: Timestamp, zone: &str) -> String {
    timestamp
        .in_tz(zone)
        .expect("zone")
        .strftime("%H:%M")
        .to_string()
}

fn minutes(clock: &str) -> i32 {
    let (hours, minutes) = clock.split_once(':').expect("HH:MM");
    hours.parse::<i32>().expect("hours") * 60 + minutes.parse::<i32>().expect("minutes")
}

fn near(clock: &str, expected: &str) -> bool {
    (minutes(clock) - minutes(expected)).abs() <= 6
}

#[test]
fn a_zone_is_placed_at_its_main_city_in_the_zone_table() {
    let stockholm = place(ZONE_TAB, "Europe/Stockholm").expect("Stockholm");
    assert!((stockholm.latitude - (59.0 + 20.0 / 60.0)).abs() < 1e-9);
    assert!((stockholm.longitude - (18.0 + 3.0 / 60.0)).abs() < 1e-9);
    let new_york = place(ZONE_TAB, "America/New_York").expect("New York");
    assert!((new_york.latitude - 40.714_166).abs() < 1e-3);
    assert!((new_york.longitude + 74.006_388).abs() < 1e-3);
    assert_eq!(place(ZONE_TAB, "Europe/Atlantis"), None);
}

#[test]
fn night_falls_at_sunset_and_day_breaks_at_sunrise() {
    let stockholm = place(ZONE_TAB, "Europe/Stockholm");
    let (scheme, sunset) = sky(&at("2026-06-21T12:00[Europe/Stockholm]"), stockholm).expect("day");
    assert_eq!(scheme, Sky::Day);
    assert!(near(&local(sunset, "Europe/Stockholm"), "22:08"));

    let (scheme, sunrise) =
        sky(&at("2026-12-21T23:30[Europe/Stockholm]"), stockholm).expect("night");
    assert_eq!(scheme, Sky::Night);
    assert_eq!(
        sunrise.in_tz("Europe/Stockholm").expect("zone").date(),
        date(2026, 12, 22)
    );
    assert!(near(&local(sunrise, "Europe/Stockholm"), "08:44"));

    let (scheme, sunrise) =
        sky(&at("2026-12-21T06:00[Europe/Stockholm]"), stockholm).expect("dawn");
    assert_eq!(scheme, Sky::Night);
    assert!(near(&local(sunrise, "Europe/Stockholm"), "08:43"));
}

#[test]
fn above_the_arctic_circle_midsummer_stays_light_and_midwinter_dark() {
    let tromso = Some(Place {
        latitude: 69.65,
        longitude: 18.96,
    });
    let (scheme, next) = sky(&at("2026-06-21T01:00[Europe/Oslo]"), tromso).expect("summer");
    assert_eq!(scheme, Sky::Day);
    assert_eq!(local(next, "Europe/Oslo"), "00:00");
    let (scheme, _) = sky(&at("2026-12-21T12:00[Europe/Oslo]"), tromso).expect("winter");
    assert_eq!(scheme, Sky::Night);
}

#[test]
fn without_a_known_place_evenings_from_seven_are_dark() {
    let (scheme, next) = sky(&at("2026-03-01T18:59[UTC]"), None).expect("day");
    assert_eq!((scheme, local(next, "UTC")), (Sky::Day, "19:00".to_owned()));
    let (scheme, next) = sky(&at("2026-03-01T19:00[UTC]"), None).expect("evening");
    assert_eq!(
        (scheme, local(next, "UTC")),
        (Sky::Night, "07:00".to_owned())
    );
}
