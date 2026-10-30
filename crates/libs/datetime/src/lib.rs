use icu_calendar::Gregorian;
use icu_datetime::fieldsets::{DE, E, MD, MDE, T, YMD};
use icu_datetime::input::{Date, DateTime, Time};
use icu_datetime::{DateTimeFormatterPreferences, FixedCalendarDateTimeFormatter};
use icu_locale_core::preferences::extensions::unicode::keywords::HourCycle;
use icu_locale_core::{Locale, locale};
use jiff::Zoned;

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
}

fn date_of(at: &Zoned) -> Option<Date<Gregorian>> {
    Date::try_new_gregorian(i32::from(at.year()), at.month() as u8, at.day() as u8).ok()
}

fn time_of(at: &Zoned) -> Option<Time> {
    Time::try_new(at.hour() as u8, at.minute() as u8, 0, 0).ok()
}

impl DateFormats {
    pub fn for_locale(locale: &Locale) -> Self {
        Self {
            preferences: locale.into(),
        }
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
        let digits = |character: char| character.is_ascii_digit() || character == ':';
        let start = formatted.find(digits).unwrap_or(0);
        let end = formatted
            .rfind(digits)
            .map_or(formatted.len(), |end| end + 1);
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
