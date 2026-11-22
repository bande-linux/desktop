use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan};

use crate::ical::freebusy::Period;

const STEP_SECONDS: i64 = 15 * 60;

pub struct Hours<'a> {
    pub zone: &'a TimeZone,
    pub day: (u8, u8),
    pub weekend: &'a dyn Fn(Weekday) -> bool,
}

impl Hours<'_> {
    pub fn of(&self, day: Date) -> Option<Period> {
        let at = |hour: u8| match hour {
            24.. => day
                .tomorrow()
                .ok()?
                .to_datetime(Time::midnight())
                .to_zoned(self.zone.clone())
                .ok(),
            hour => day
                .to_datetime(Time::new(i8::try_from(hour).ok()?, 0, 0, 0).ok()?)
                .to_zoned(self.zone.clone())
                .ok(),
        };
        let (start, end) = (at(self.day.0)?, at(self.day.1)?);
        (end > start).then(|| (start.timestamp(), end.timestamp()))
    }
}

fn round_up(at: Timestamp) -> Timestamp {
    let seconds = at.as_second();
    let rounded = seconds.div_euclid(STEP_SECONDS) * STEP_SECONDS;
    let rounded = match rounded < seconds || at.subsec_nanosecond() > 0 {
        true => rounded + STEP_SECONDS,
        false => rounded,
    };
    Timestamp::from_second(rounded).unwrap_or(at)
}

/// The first `count` starts, from `after` on and within `days` working days' hours, of a
/// meeting of `length` that overlaps none of the `busy` periods.
pub fn free_slots(
    busy: &[Period],
    length: SignedDuration,
    after: Timestamp,
    hours: &Hours,
    days: i64,
    count: usize,
) -> Vec<Timestamp> {
    let mut found = Vec::new();
    if length <= SignedDuration::ZERO {
        return found;
    }
    let first = after.to_zoned(hours.zone.clone()).date();
    for offset in 0..days {
        let Ok(day) = first.checked_add(offset.days()) else {
            break;
        };
        let Some((open, close)) = hours.of(day).filter(|_| !(hours.weekend)(day.weekday())) else {
            continue;
        };
        let mut start = round_up(open.max(after));
        while let Ok(end) = start.checked_add(length) {
            if end > close || found.len() == count {
                break;
            }
            match busy
                .iter()
                .filter(|(from, to)| *from < end && *to > start)
                .map(|(_, to)| *to)
                .max()
            {
                Some(free_from) => start = round_up(free_from),
                None => {
                    found.push(start);
                    start = end;
                }
            }
        }
    }
    found
}
