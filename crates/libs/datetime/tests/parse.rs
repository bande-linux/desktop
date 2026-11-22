use bande_datetime::{DateFormats, posix_locale};
use jiff::civil::{date, time};

#[test]
fn a_typed_date_and_time_are_read_in_the_locales_own_order() {
    let us = DateFormats::for_locale(&posix_locale("en_US.UTF-8"));
    let british = DateFormats::for_locale(&posix_locale("en_GB.UTF-8"));
    let swedish = DateFormats::for_locale(&posix_locale("sv_SE.UTF-8"));
    assert_eq!(us.parse_date("03/04/2026"), Some(date(2026, 3, 4)));
    assert_eq!(british.parse_date("03/04/2026"), Some(date(2026, 4, 3)));
    assert_eq!(swedish.parse_date("2026-03-04"), Some(date(2026, 3, 4)));
    assert_eq!(british.parse_date("2026-03-04"), Some(date(2026, 3, 4)));
    let day = date(2026, 11, 22);
    for formats in [&us, &british, &swedish] {
        assert_eq!(formats.parse_date(&formats.numeric_date(day)), Some(day));
    }
    assert_eq!(us.parse_time("3:30 PM"), Some(time(15, 30, 0, 0)));
    assert_eq!(us.parse_time("12:00 AM"), Some(time(0, 0, 0, 0)));
    assert_eq!(swedish.parse_time("15.30"), Some(time(15, 30, 0, 0)));
    let afternoon = date(2026, 1, 1)
        .at(15, 30, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .expect("a valid moment");
    assert_eq!(
        us.parse_time(&us.clock_time(&afternoon)),
        Some(time(15, 30, 0, 0))
    );
}
