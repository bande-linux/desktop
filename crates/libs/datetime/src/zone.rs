use std::path::Path;

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoneSuggestion {
    Zone(String),
    Choose,
}

const LOCALTIME: &str = "/etc/localtime";
const ZONEINFO: &str = "zoneinfo/";

pub fn city_name(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

pub fn local_zone() -> TimeZone {
    if std::env::var_os("TZ").is_some() {
        return TimeZone::system();
    }
    zone_at(Path::new(LOCALTIME))
}

pub fn now() -> Zoned {
    Timestamp::now().to_zoned(local_zone())
}

fn zone_at(link: &Path) -> TimeZone {
    std::fs::read_link(link)
        .ok()
        .and_then(|target| {
            let target = target.to_str()?;
            let (_, name) = target.rsplit_once(ZONEINFO)?;
            jiff::tz::db().get(name).ok()
        })
        .unwrap_or_else(TimeZone::system)
}

/// The time zone to suggest for a computer in `country` (two letters) whose clock shows `current`:
/// none when one of the country's zones already shows the same time at `at`, the country's first
/// zone in `zone_tab` when all its zones show one time, or else a choice among them.
pub fn suggested_zone(
    zone_tab: &str,
    country: &str,
    current: &TimeZone,
    at: Timestamp,
) -> Option<ZoneSuggestion> {
    let zones: Vec<(&str, jiff::tz::Offset)> = zone_tab
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let mut fields = line.split('\t');
            (fields.next()? == country).then_some(())?;
            let name = fields.nth(1)?;
            Some((name, jiff::tz::db().get(name).ok()?.to_offset(at)))
        })
        .collect();
    let here = current.to_offset(at);
    let (first, offset) = *zones.first()?;
    if zones.iter().any(|(_, zone)| *zone == here) {
        None
    } else if zones.iter().all(|(_, zone)| *zone == offset) {
        Some(ZoneSuggestion::Zone(first.to_owned()))
    } else {
        Some(ZoneSuggestion::Choose)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const ZONES: &str = "# comment\nDE\t+5230+01322\tEurope/Berlin\tmost of Germany\nDE\t+4742+00841\tEurope/Busingen\tBusingen\nGB\t+513030-0000731\tEurope/London\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\nUS\t+340308-1181434\tAmerica/Los_Angeles\tPacific\n";

    #[test]
    fn a_zone_is_suggested_only_when_the_clock_shows_another_time_than_the_country() {
        let at: Timestamp = "2026-07-01T12:00:00Z".parse().unwrap();
        let zone = |name: &str| jiff::tz::db().get(name).unwrap();
        let stockholm = zone("Europe/Stockholm");
        assert_eq!(suggested_zone(ZONES, "DE", &stockholm, at), None);
        assert_eq!(
            suggested_zone(ZONES, "GB", &stockholm, at),
            Some(ZoneSuggestion::Zone("Europe/London".to_owned()))
        );
        assert_eq!(
            suggested_zone(ZONES, "US", &stockholm, at),
            Some(ZoneSuggestion::Choose)
        );
        assert_eq!(
            suggested_zone(ZONES, "US", &zone("America/Los_Angeles"), at),
            None
        );
        assert_eq!(suggested_zone(ZONES, "SE", &stockholm, at), None);
    }
}
