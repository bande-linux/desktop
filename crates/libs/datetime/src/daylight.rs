use jiff::civil::{Date, Time};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

pub const ZONE_TAB: &str = "/usr/share/zoneinfo/zone.tab";

const UNIX_EPOCH_JULIAN: f64 = 2_440_587.5;
const J2000: f64 = 2_451_545.0;
const SECONDS_PER_DAY: f64 = 86_400.0;
const SUN_ALTITUDE_AT_RISE: f64 = -0.833;
const EARTH_TILT: f64 = 23.4397;
const MORNING: Time = Time::constant(7, 0, 0, 0);
const EVENING: Time = Time::constant(19, 0, 0, 0);
const NOON: Time = Time::constant(12, 0, 0, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sky {
    Day,
    Night,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub latitude: f64,
    pub longitude: f64,
}

fn degrees(text: &str, whole_digits: usize) -> Option<f64> {
    let sign = match text.as_bytes().first()? {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };
    let digits = text.get(1..)?;
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let whole: f64 = digits.get(..whole_digits)?.parse().ok()?;
    let minutes: f64 = digits.get(whole_digits..whole_digits + 2)?.parse().ok()?;
    let seconds: f64 = digits
        .get(whole_digits + 2..)
        .filter(|rest| !rest.is_empty())
        .map_or(Some(0.0), |rest| rest.parse().ok())?;
    Some(sign * (whole + minutes / 60.0 + seconds / 3600.0))
}

fn coordinates(text: &str) -> Option<Place> {
    let split = text.get(1..)?.find(['+', '-'])? + 1;
    let (latitude, longitude) = text.split_at(split);
    Some(Place {
        latitude: degrees(latitude, 2)?,
        longitude: degrees(longitude, 3)?,
    })
}

pub fn place(zone_tab: &str, zone: &str) -> Option<Place> {
    zone_tab
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| line.split('\t'))
        .find_map(|mut fields| {
            let position = fields.nth(1)?;
            (fields.next()? == zone).then(|| coordinates(position))?
        })
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Daylight {
    Between(Timestamp, Timestamp),
    AlwaysUp,
    AlwaysDown,
}

fn timestamp(julian: f64) -> Option<Timestamp> {
    Timestamp::from_second(((julian - UNIX_EPOCH_JULIAN) * SECONDS_PER_DAY).round() as i64).ok()
}

fn daylight(noon: Timestamp, place: Place) -> Option<Daylight> {
    let julian = noon.as_second() as f64 / SECONDS_PER_DAY + UNIX_EPOCH_JULIAN;
    let day = (julian - J2000 + 0.0008).ceil();
    let mean_noon = day - place.longitude / 360.0;
    let anomaly = (357.5291 + 0.985_600_28 * mean_noon).rem_euclid(360.0);
    let radians = anomaly.to_radians();
    let center =
        1.9148 * radians.sin() + 0.02 * (2.0 * radians).sin() + 0.0003 * (3.0 * radians).sin();
    let ecliptic = (anomaly + center + 180.0 + 102.9372)
        .rem_euclid(360.0)
        .to_radians();
    let transit = J2000 + mean_noon + 0.0053 * radians.sin() - 0.0069 * (2.0 * ecliptic).sin();
    let declination = (ecliptic.sin() * EARTH_TILT.to_radians().sin()).asin();
    let latitude = place.latitude.to_radians();
    let hour_angle = (SUN_ALTITUDE_AT_RISE.to_radians().sin() - latitude.sin() * declination.sin())
        / (latitude.cos() * declination.cos());
    Some(match hour_angle {
        cosine if cosine > 1.0 => Daylight::AlwaysDown,
        cosine if cosine < -1.0 => Daylight::AlwaysUp,
        cosine => {
            let half_day = cosine.acos().to_degrees() / 360.0;
            Daylight::Between(
                timestamp(transit - half_day)?,
                timestamp(transit + half_day)?,
            )
        }
    })
}

fn at(date: Date, time: Time, zone: &TimeZone) -> Option<Timestamp> {
    date.to_datetime(time)
        .to_zoned(zone.clone())
        .ok()
        .map(|zoned| zoned.timestamp())
}

fn day(date: Date, zone: &TimeZone, place: Option<Place>) -> Option<Daylight> {
    match place {
        Some(place) => daylight(at(date, NOON, zone)?, place),
        None => Some(Daylight::Between(
            at(date, MORNING, zone)?,
            at(date, EVENING, zone)?,
        )),
    }
}

pub fn sky(now: &Zoned, place: Option<Place>) -> Option<(Sky, Timestamp)> {
    let zone = now.time_zone();
    let today = now.date();
    let tomorrow = today.tomorrow().ok()?;
    let instant = now.timestamp();
    let next_day = at(tomorrow, Time::midnight(), zone)?;
    Some(match day(today, zone, place)? {
        Daylight::AlwaysUp => (Sky::Day, next_day),
        Daylight::AlwaysDown => (Sky::Night, next_day),
        Daylight::Between(rise, _) if instant < rise => (Sky::Night, rise),
        Daylight::Between(_, set) if instant < set => (Sky::Day, set),
        Daylight::Between(..) => match day(tomorrow, zone, place)? {
            Daylight::Between(rise, _) => (Sky::Night, rise),
            Daylight::AlwaysUp | Daylight::AlwaysDown => (Sky::Night, next_day),
        },
    })
}

pub fn system_place() -> Option<Place> {
    let zone = TimeZone::system();
    let name = zone.iana_name()?;
    place(&std::fs::read_to_string(ZONE_TAB).ok()?, name)
}
