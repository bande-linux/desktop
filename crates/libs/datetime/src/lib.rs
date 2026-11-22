pub mod daylight;
mod minute;
mod month;
pub mod timedate;
mod zone;

use icu_calendar::Gregorian;
use icu_calendar::week::WeekInformation;
use std::fmt;

use icu_datetime::fieldsets::{DE, E, M, MD, MDE, T, YM, YMD};
use icu_datetime::input::{Date, DateTime, Time};
use icu_datetime::options::YearStyle;
use icu_datetime::parts::{DAY, MONTH, YEAR};
use icu_datetime::{DateTimeFormatterPreferences, FixedCalendarDateTimeFormatter};
use icu_locale_core::preferences::extensions::unicode::keywords::HourCycle;
use icu_locale_core::{Locale, locale};
use jiff::Zoned;
use writeable::{Part, PartsWrite, Writeable};

pub use minute::{ClockChanges, MinuteTimer};
pub use month::{Month, Week};
pub use zone::{ZoneSuggestion, city_name, local_zone, now, suggested_zone};

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

/// The locale named by the first set of `variables`, such as `["LC_ALL", "LC_NUMERIC", "LANG"]`.
pub fn env_locale(variables: &[&str]) -> Locale {
    let value = variables
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
        .unwrap_or_default();
    posix_locale(&value)
}

#[derive(Debug, Clone)]
pub struct DateFormats {
    preferences: DateTimeFormatterPreferences,
    first_weekday: jiff::civil::Weekday,
    weekend: Vec<jiff::civil::Weekday>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Year,
    Month,
    Day,
}

const ISO_ORDER: [Field; 3] = [Field::Year, Field::Month, Field::Day];

fn full_year(digits: &str) -> Option<i16> {
    let year: i16 = digits.parse().ok()?;
    Some(match digits.len() {
        1 | 2 => 2000 + year,
        _ => year,
    })
}

fn hours_minutes(body: &str) -> Option<(i8, i8)> {
    match body.split_once([':', '.', 'h']) {
        Some((hours, minutes)) => Some((hours.trim().parse().ok()?, minutes.trim().parse().ok()?)),
        None if body.len() > 2 => {
            let split = body.len() - 2;
            Some((
                body.get(..split)?.parse().ok()?,
                body.get(split..)?.parse().ok()?,
            ))
        }
        None => Some((body.parse().ok()?, 0)),
    }
}

#[derive(Default)]
struct Parts(Vec<(Option<Part>, String)>);

impl fmt::Write for Parts {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        match self.0.last_mut() {
            Some((None, literal)) => literal.push_str(text),
            _ => self.0.push((None, text.to_owned())),
        }
        Ok(())
    }
}

impl PartsWrite for Parts {
    type SubPartsWrite = Self;

    fn with_part(
        &mut self,
        part: Part,
        mut write: impl FnMut(&mut Self) -> fmt::Result,
    ) -> fmt::Result {
        let mut inner = Self::default();
        write(&mut inner)?;
        let text = inner.0.into_iter().map(|(_, text)| text).collect();
        self.0.push((Some(part), text));
        Ok(())
    }
}

fn without_month(Parts(mut parts): Parts) -> Option<String> {
    let month = parts.iter().position(|(part, _)| *part == Some(MONTH))?;
    let is_literal = |parts: &[(Option<Part>, String)], index: usize| {
        parts.get(index).is_some_and(|(part, _)| part.is_none())
    };
    let field_before = parts[..month].iter().any(|(part, _)| part.is_some());
    let field_after = parts[month + 1..].iter().any(|(part, _)| part.is_some());
    if is_literal(&parts, month + 1) && (!field_after || !field_before) {
        parts.remove(month + 1);
    }
    if month > 0 && is_literal(&parts, month - 1) {
        match field_after {
            true if field_before => {
                parts.remove(month - 1);
            }
            true => {}
            false => {
                let literal = &mut parts[month - 1].1;
                let attached = literal.find(char::is_whitespace).unwrap_or(literal.len());
                literal.truncate(attached);
            }
        }
    }
    parts.retain(|(part, _)| *part != Some(MONTH));
    let text: String = parts.into_iter().map(|(_, text)| text).collect();
    Some(text.trim().to_owned())
}

impl DateFormats {
    pub fn for_locale(locale: &Locale) -> Self {
        let weekday = |day: icu_calendar::types::Weekday| {
            jiff::civil::Weekday::from_monday_one_offset(day as i8).ok()
        };
        let week = WeekInformation::try_new(locale.into()).ok();
        let first_weekday = week
            .as_ref()
            .and_then(|week| weekday(week.first_weekday))
            .unwrap_or(jiff::civil::Weekday::Monday);
        let weekend = week.map_or_else(
            || vec![jiff::civil::Weekday::Saturday, jiff::civil::Weekday::Sunday],
            |week| week.weekend().filter_map(weekday).collect(),
        );
        Self {
            preferences: locale.into(),
            first_weekday,
            weekend,
        }
    }

    pub fn first_weekday(&self) -> jiff::civil::Weekday {
        self.first_weekday
    }

    /// Whether `day` is a weekend day where this locale is used.
    pub fn is_weekend(&self, day: jiff::civil::Weekday) -> bool {
        self.weekend.contains(&day)
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

    pub fn month_name(&self, month: i8) -> String {
        let day = jiff::civil::date(2000, month, 1);
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(self.preferences, M::long())
            .ok()
            .zip(gregorian(day))
            .map(|(formatter, date)| formatter.format(&date).to_string())
            .unwrap_or_else(|| day.strftime("%B").to_string())
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
        Self::for_locale(&env_locale(&LOCALE_VARIABLES))
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
            .map_or(formatted.len(), |(end, character)| {
                end + character.len_utf8()
            });
        let period = format!("{} {}", &formatted[..start], &formatted[end..]);
        let period = period.trim_matches(|character: char| character.is_whitespace());
        (
            formatted[start..end].to_owned(),
            (!period.is_empty()).then(|| period.to_owned()),
        )
    }

    pub fn short_day(&self, at: &Zoned) -> String {
        let date = date_of(at);
        let with_month = FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(
            self.preferences,
            MDE::medium(),
        )
        .ok()
        .zip(date)
        .and_then(|(formatter, date)| {
            let mut parts = Parts::default();
            formatter.format(&date).write_to_parts(&mut parts).ok()?;
            without_month(parts)
        });
        with_month
            .or_else(|| {
                FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(
                    self.preferences,
                    DE::short(),
                )
                .ok()
                .zip(date)
                .map(|(formatter, date)| formatter.format(&date).to_string())
            })
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

    pub fn date_time(&self, at: &Zoned, with_year: bool) -> String {
        let (time, period) = self.time(at);
        let period = period
            .map(|period| format!(" {period}"))
            .unwrap_or_default();
        format!("{}, {time}{period}", self.date(at, with_year))
    }

    pub fn clock_time(&self, at: &Zoned) -> String {
        let (time, period) = self.time(at);
        period.map_or(time.clone(), |period| format!("{time} {period}"))
    }

    pub fn numeric_date(&self, day: jiff::civil::Date) -> String {
        FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(
            self.preferences,
            YMD::short().with_year_style(YearStyle::Full),
        )
        .ok()
        .zip(gregorian(day))
        .map(|(formatter, date)| formatter.format(&date).to_string())
        .unwrap_or_else(|| day.strftime("%Y-%m-%d").to_string())
    }

    fn date_order(&self) -> [Field; 3] {
        let fields = FixedCalendarDateTimeFormatter::<Gregorian, _>::try_new(
            self.preferences,
            YMD::short().with_year_style(YearStyle::Full),
        )
        .ok()
        .zip(gregorian(jiff::civil::date(2001, 2, 3)))
        .and_then(|(formatter, date)| {
            let mut parts = Parts::default();
            formatter.format(&date).write_to_parts(&mut parts).ok()?;
            let fields: Vec<Field> = parts
                .0
                .iter()
                .filter_map(|(part, _)| match *part {
                    Some(YEAR) => Some(Field::Year),
                    Some(MONTH) => Some(Field::Month),
                    Some(DAY) => Some(Field::Day),
                    _ => None,
                })
                .collect();
            <[Field; 3]>::try_from(fields).ok()
        });
        fields.unwrap_or(ISO_ORDER)
    }

    /// Reads a numeric date typed in this locale's field order, or in ISO order when it starts
    /// with a full year.
    pub fn parse_date(&self, text: &str) -> Option<jiff::civil::Date> {
        let numbers: Vec<&str> = text
            .split(|character: char| !character.is_ascii_digit())
            .filter(|number| !number.is_empty())
            .collect();
        let [first, second, third] = numbers.as_slice() else {
            return None;
        };
        let order = match first.len() > 2 {
            true => ISO_ORDER,
            false => self.date_order(),
        };
        let (mut year, mut month, mut day) = (None, None, None);
        for (field, number) in order.into_iter().zip([first, second, third]) {
            match field {
                Field::Year => year = full_year(number),
                Field::Month => month = number.parse().ok(),
                Field::Day => day = number.parse().ok(),
            }
        }
        jiff::civil::Date::new(year?, month?, day?).ok()
    }

    /// Reads a time of day typed with this locale's or the English day periods, or on a 24-hour
    /// clock.
    pub fn parse_time(&self, text: &str) -> Option<jiff::civil::Time> {
        let lower = text.trim().to_lowercase();
        let period = |hour: i8| {
            jiff::civil::date(2026, 1, 1)
                .at(hour, 0, 0, 0)
                .to_zoned(jiff::tz::TimeZone::UTC)
                .ok()
                .and_then(|at| self.time(&at).1)
                .map(|period| period.to_lowercase())
        };
        let periods = [
            (period(1), false),
            (period(13), true),
            (Some("am".to_owned()), false),
            (Some("pm".to_owned()), true),
        ];
        let (body, afternoon) = periods
            .iter()
            .filter_map(|(marker, afternoon)| Some((marker.as_deref()?, *afternoon)))
            .find_map(|(marker, afternoon)| {
                lower
                    .strip_suffix(marker)
                    .or_else(|| lower.strip_prefix(marker))
                    .map(|body| (body.trim(), Some(afternoon)))
            })
            .unwrap_or((lower.as_str(), None));
        let (hours, minutes) = hours_minutes(body)?;
        let hours = match afternoon {
            Some(_) if !(1..=12).contains(&hours) => return None,
            Some(true) if hours < 12 => hours + 12,
            Some(false) if hours == 12 => 0,
            _ => hours,
        };
        jiff::civil::Time::new(hours, minutes, 0, 0).ok()
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
