#![allow(clippy::expect_used)]

use icu_locale_core::locale;
use jiff::Zoned;
use unbound_datetime::{DateFormats, posix_locale};

fn at(text: &str) -> Zoned {
    format!("{text}[Europe/Stockholm]")
        .parse()
        .expect("a valid time")
}

fn formats(posix: &str) -> DateFormats {
    DateFormats::for_locale(&posix_locale(posix))
}

#[test]
fn posix_locale_names_become_language_tags_and_c_means_american_english() {
    assert_eq!(posix_locale("de_AT.UTF-8@euro"), locale!("de-AT"));
    assert_eq!(posix_locale("sv_SE"), locale!("sv-SE"));
    assert_eq!(posix_locale("C.UTF-8"), locale!("en-US"));
    assert_eq!(posix_locale("POSIX"), locale!("en-US"));
    assert_eq!(posix_locale(""), locale!("en-US"));
    assert_eq!(posix_locale("not a locale!"), locale!("en-US"));
}

#[test]
fn the_clock_follows_the_order_names_and_hour_cycle_of_the_locale() {
    let afternoon = at("2026-10-30T14:05:00");
    assert_eq!(
        formats("en_US.UTF-8").clock(&afternoon),
        "Fri, Oct 30, 2:05\u{202f}PM"
    );
    assert_eq!(
        formats("en_GB.UTF-8").clock(&afternoon),
        "Fri 30 Oct, 14:05"
    );
    assert_eq!(
        formats("de_DE.UTF-8").clock(&afternoon),
        "Fr., 30. Okt., 14:05"
    );
}

#[test]
fn weekdays_and_dates_are_written_in_the_locale() {
    let day = at("2026-03-02T09:00:00");
    assert_eq!(formats("en_US.UTF-8").weekday(&day), "Monday");
    assert_eq!(formats("fr_FR.UTF-8").weekday(&day), "lundi");
    assert_eq!(formats("en_US.UTF-8").date(&day, false), "Mar 2");
    assert_eq!(formats("en_US.UTF-8").date(&day, true), "Mar 2, 2026");
    assert_eq!(formats("de_DE.UTF-8").date(&day, true), "02.03.2026");
    assert_eq!(formats("en_US.UTF-8").long_date(&day), "Monday, March 2");
    assert_eq!(formats("de_DE.UTF-8").long_date(&day), "Montag, 2. März");
}

#[test]
fn the_bar_clock_stacks_the_time_over_the_day_with_any_day_period_apart() {
    let afternoon = at("2026-10-30T14:05:00");
    assert_eq!(
        formats("en_GB.UTF-8").time(&afternoon),
        ("14:05".to_owned(), None)
    );
    assert_eq!(
        formats("en_US.UTF-8").time(&afternoon),
        ("2:05".to_owned(), Some("PM".to_owned()))
    );
    assert_eq!(formats("en_GB.UTF-8").short_day(&afternoon), "Fri 30");
}
