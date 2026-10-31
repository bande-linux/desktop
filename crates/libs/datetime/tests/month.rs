#![allow(clippy::expect_used)]

use jiff::Zoned;
use jiff::civil::{Weekday, date};
use unbound_datetime::{DateFormats, Month, posix_locale};

fn formats(posix: &str) -> DateFormats {
    DateFormats::for_locale(&posix_locale(posix))
}

fn halloween() -> Zoned {
    "2026-10-31T12:00:00[Europe/Stockholm]"
        .parse()
        .expect("a valid time")
}

#[test]
fn american_weeks_start_on_sunday_without_numbers() {
    let american = formats("en_US.UTF-8");
    assert_eq!(american.first_weekday(), Weekday::Sunday);

    let october = american.month(&halloween());
    assert_eq!(october.first, date(2026, 10, 1));
    assert_eq!(october.weeks[0].days[0], date(2026, 9, 27));
    assert_eq!(october.weeks[0].number, None);
    assert_eq!(american.month_title(october.first), "October 2026");
    assert_eq!(american.short_weekday(october.weeks[0].days[0]), "Sun");
}

#[test]
fn swedish_weeks_start_on_monday_and_carry_iso_week_numbers() {
    let swedish = formats("sv_SE.UTF-8");
    assert_eq!(swedish.first_weekday(), Weekday::Monday);

    let october = swedish.month(&halloween());
    assert_eq!(october.weeks[0].days[0], date(2026, 9, 28));
    let numbers: Vec<Option<i8>> = october.weeks.iter().map(|week| week.number).collect();
    assert_eq!(
        numbers,
        [Some(40), Some(41), Some(42), Some(43), Some(44), Some(45)]
    );
    assert_eq!(swedish.month_title(october.first), "oktober 2026");
}

#[test]
fn every_month_shows_six_weeks_so_the_grid_never_jumps_and_the_new_year_is_week_one() {
    let january = Month::containing(date(2027, 1, 15), Weekday::Monday);
    assert_eq!(january.weeks.len(), 6);
    assert_eq!(january.weeks[0].days[0], date(2026, 12, 28));
    assert_eq!(january.weeks[0].number, Some(53));
    assert!(!january.contains(january.weeks[0].days[0]));
    assert!(january.contains(january.weeks[0].days[4]));

    let december = january.shifted(-1);
    assert_eq!(december.first, date(2026, 12, 1));
    assert_eq!(december.weeks[0].days[0], date(2026, 11, 30));
    assert_eq!(december.weeks.len(), 6);
    assert_eq!(december.shifted(13).first, date(2028, 1, 1));
}
