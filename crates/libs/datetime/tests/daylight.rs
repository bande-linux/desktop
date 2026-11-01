#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_datetime::daylight::{Place, Sky, place, sky};
use jiff::{Timestamp, Zoned};

const ZONE_TAB: &str = "# comment\nSE\t+5920+01803\tEurope/Stockholm\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\n";

fn at(text: &str) -> Zoned {
    text.parse().expect("zoned")
}

fn minutes(timestamp: Timestamp, zone: &str) -> (i8, i32) {
    let local = timestamp.in_tz(zone).expect("zone");
    (
        local.day(),
        i32::from(local.hour()) * 60 + i32::from(local.minute()),
    )
}

fn near((day, minute): (i8, i32), expected_day: i8, (hour, at): (i32, i32)) -> bool {
    day == expected_day && (minute - (hour * 60 + at)).abs() <= 6
}

#[test]
fn the_sun_rises_and_sets_on_the_right_day_east_and_west_of_greenwich_and_past_the_arctic_circle() {
    let stockholm = place(ZONE_TAB, "Europe/Stockholm");
    let (scheme, sunset) = sky(&at("2026-06-21T12:00[Europe/Stockholm]"), stockholm).expect("day");
    assert_eq!(scheme, Sky::Day);
    assert!(near(minutes(sunset, "Europe/Stockholm"), 21, (22, 8)));
    let (scheme, sunrise) =
        sky(&at("2026-12-21T23:30[Europe/Stockholm]"), stockholm).expect("night");
    assert_eq!(scheme, Sky::Night);
    assert!(near(minutes(sunrise, "Europe/Stockholm"), 22, (8, 44)));

    let new_york = place(ZONE_TAB, "America/New_York");
    let (scheme, sunset) = sky(&at("2026-11-01T10:00[America/New_York]"), new_york).expect("day");
    assert_eq!(scheme, Sky::Day);
    assert!(near(minutes(sunset, "America/New_York"), 1, (16, 52)));
    let auckland = Some(Place {
        latitude: -36.85,
        longitude: 174.77,
    });
    let (_, sunset) = sky(&at("2026-11-01T10:00[Pacific/Auckland]"), auckland).expect("day");
    assert_eq!(minutes(sunset, "Pacific/Auckland").0, 1);

    let tromso = Some(Place {
        latitude: 69.65,
        longitude: 18.96,
    });
    let (scheme, _) = sky(&at("2026-06-21T01:00[Europe/Oslo]"), tromso).expect("summer");
    assert_eq!(scheme, Sky::Day);
    let (scheme, _) = sky(&at("2026-12-21T12:00[Europe/Oslo]"), tromso).expect("winter");
    assert_eq!(scheme, Sky::Night);
}
