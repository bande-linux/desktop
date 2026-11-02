pub mod daylight;
mod minute;
mod month;
pub mod timedate;
mod zone;

use icu_calendar::Gregorian;
use icu_calendar::week::WeekInformation;
use icu_datetime::fieldsets::{DE, E, MD, MDE, T, YM, YMD};
use icu_datetime::input::{Date, DateTime, Time};
use icu_datetime::{DateTimeFormatterPreferences, FixedCalendarDateTimeFormatter};
use icu_locale_core::preferences::extensions::unicode::keywords::HourCycle;
use icu_locale_core::{Locale, locale};
use jiff::Zoned;

pub use minute::{ClockChanges, MinuteTimer};
pub use month::{Month, Week};
pub use zone::{local_zone, now};

const LOCALE_VARIABLES: [&str; 3] = ["LC_ALL", "LC_TIME", "LANG"];
const FALLBACK: Locale = locale!("en-US");

pub fn posix_locale(value: &str) -> Locale {
    let language = value
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-");
    if language.is_empty() || language == "C" || language == "POSIX" {
        return FALLBACK;
    }
    language.parse().unwrap_or(FALLBACK)
}

#[derive(Debug, Clone)]
pub struct DateFormats {
    preferences: DateTimeFormatterPreferences,
    first_weekday: jiff::civil::Weekday,
}

fn gregorian(day: jiff::civil::Date) -> Option<Date<Gregorian>> {
    Date::try_new_gregorian(i32::from(day.year()), day.month() as u8, day.day() as u8).ok()
}

fn date_of(at: &Zoned) -> Option<Date<Gregorian>> {
    gregorian(at.date())
}

fn time_of(at: &Zoned) -> Option<Time> {
    Time::try_new(at.hour() as u8, at.minute() as u8, 0, 0).ok()
}

impl DateFormats {
    pub fn for_locale(locale: &Locale) -> Self {
        let first_weekday =
            WeekInformation::try_new(locale.into()).map_or(jiff::civil::Weekday::Monday, |week| {
                jiff::civil::Weekday::from_monday_one_offset(week.first_weekday as i8)
                    .unwrap_or(jiff::civil::Weekday::Monday)
            });
        Self {
            preferences: locale.into(),
            first_weekday,
        }
    }

    pub fn first_weekday(&self) -> jiff::civil::Weekday {
        self.first_weekday
    }

    pub fn month(&self, at: &Zoned) -> Month {
        Month::containing(at.date(), self.first_weekday)
    }

    pub fn month_title(&self, first: jiff::civil::Date) -> String {
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, YM::long())
            .ok()
            .zip(gregorian(first))
            .map(|(formatter, date)| formatter.format(&date).to_string())
            .unwrap_or_else(|| first.strftime("%B %Y").to_string())
    }

    pub fn short_weekday(&self, day: jiff::civil::Date) -> String {
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, E::short())
            .ok()
            .zip(gregorian(day))
            .map(|(formatter, date)| formatter.format(&date).to_string())
            .unwrap_or_else(|| day.strftime("%a").to_string())
    }

    pub fn with_clock_format(mut self, format: &str) -> Self {
        self.preferences.hour_cycle = match format {
            "24-hour" => Some(HourCycle::Clock24),
            "12-hour" => Some(HourCycle::Clock12),
            _ => None,
        };
        self
    }

    pub fn uses_24_hour_clock(&self) -> bool {
        let afternoon = jiff::civil::date(2026, 1, 1)
            .at(13, 0, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC);
        afternoon.is_ok_and(|afternoon| self.clock(&afternoon).contains("13"))
    }

    pub fn from_env() -> Self {
        let value = LOCALE_VARIABLES
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        Self::for_locale(&posix_locale(&value))
    }

    pub fn clock(&self, at: &Zoned) -> String {
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(
            self.preferences,
            MDE::medium().with_time_hm(),
        )
        .ok()
        .zip(date_of(at).zip(time_of(at)))
        .map(|(formatter, (date, time))| formatter.format(&DateTime { date, time }).to_string())
        .unwrap_or_else(|| at.strftime("%a %-d %b %H:%M").to_string())
    }

    pub fn time(&self, at: &Zoned) -> (String, Option<String>) {
        let formatted =
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, T::hm())
                .ok()
                .zip(date_of(at).zip(time_of(at)))
                .map(|(formatter, (date, time))| {
                    formatter.format(&DateTime { date, time }).to_string()
                })
                .unwrap_or_else(|| at.strftime("%H:%M").to_string());
        let digits = |character: char| character.is_numeric() || character == ':';
        let start = formatted.find(digits).unwrap_or(0);
        let end = formatted
            .char_indices()
            .rfind(|(_, character)| digits(*character))
            .map_or(formatted.len(), |(end, character)| end + character.len_utf8());
        let period = format!("{} {}", &formatted[..start], &formatted[end..]);
        let period = period.trim_matches(|character: char| character.is_whitespace());
        (
            formatted[start..end].to_owned(),
            (!period.is_empty()).then(|| period.to_owned()),
        )
    }

    pub fn short_day(&self, at: &Zoned) -> String {
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, DE::short())
            .ok()
            .zip(date_of(at))
            .map(|(formatter, date)| formatter.format(&date).to_string())
            .unwrap_or_else(|| at.strftime("%a %-d").to_string())
    }

    pub fn weekday(&self, at: &Zoned) -> String {
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, E::long())
            .ok()
            .zip(date_of(at))
            .map(|(formatter, date)| formatter.format(&date).to_string())
            .unwrap_or_else(|| at.strftime("%A").to_string())
    }

    pub fn long_date(&self, at: &Zoned) -> String {
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, MDE::long())
            .ok()
            .zip(date_of(at))
            .map(|(formatter, date)| formatter.format(&date).to_string())
            .unwrap_or_else(|| at.strftime("%A %-d %B").to_string())
    }

    pub fn date(&self, at: &Zoned, with_year: bool) -> String {
        let formatted = if with_year {
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, YMD::medium())
                .ok()
                .zip(date_of(at))
                .map(|(formatter, date)| formatter.format(&date).to_string())
        } else {
            FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, MD::medium())
                .ok()
                .zip(date_of(at))
                .map(|(formatter, date)| formatter.format(&date).to_string())
        };
        formatted.unwrap_or_else(|| at.strftime("%-d %b %Y").to_string())
    }
}
