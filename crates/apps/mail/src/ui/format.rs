use bande_datetime::DateFormats;
use bande_l10n::tr;
use jiff::Timestamp;
use jiff::tz::TimeZone;

pub fn list_date(seconds: i64, now: Timestamp, zone: &TimeZone, formats: &DateFormats) -> String {
    let Ok(time) = Timestamp::from_second(seconds) else {
        return String::new();
    };
    let (time, now) = (time.to_zoned(zone.clone()), now.to_zoned(zone.clone()));
    let (day, today) = (time.date(), now.date());
    if day == today {
        return formats.clock_time(&time);
    }
    if today.yesterday().is_ok_and(|yesterday| yesterday == day) {
        return tr!("Yesterday");
    }
    let days = (today - day).get_days();
    if (0..7).contains(&days) {
        return formats.weekday(&time);
    }
    formats.date(&time, day.year() != today.year())
}

pub fn day_group(seconds: i64, now: Timestamp, zone: &TimeZone, formats: &DateFormats) -> String {
    let Ok(time) = Timestamp::from_second(seconds) else {
        return tr!("Unknown Date");
    };
    let (day, today) = (
        time.to_zoned(zone.clone()).date(),
        now.to_zoned(zone.clone()).date(),
    );
    match (today - day).get_days() {
        days if days <= 0 => tr!("Today"),
        1 => tr!("Yesterday"),
        2..=6 if day.iso_week_date().week() == today.iso_week_date().week() => {
            tr!("This Week")
        }
        _ if day.year() == today.year() => formats.month_name(day.month()),
        _ => formats.month_title(day.first_of_month()),
    }
}

pub fn full_date(seconds: i64, zone: &TimeZone, formats: &DateFormats) -> String {
    Timestamp::from_second(seconds)
        .map(|time| formats.date_time(&time.to_zoned(zone.clone()), true))
        .unwrap_or_default()
}

pub fn initials(name: &str) -> String {
    if name.contains('@') {
        return name.chars().take(1).flat_map(char::to_uppercase).collect();
    }
    let words = name
        .split(|character: char| character.is_whitespace() || character == '@' || character == '.')
        .filter(|word| word.chars().next().is_some_and(char::is_alphabetic))
        .collect::<Vec<_>>();
    match words.as_slice() {
        [] => String::new(),
        [only] => only.chars().take(1).flat_map(char::to_uppercase).collect(),
        [first, .., last] => [first, last]
            .iter()
            .filter_map(|word| word.chars().next())
            .flat_map(char::to_uppercase)
            .collect(),
    }
}
