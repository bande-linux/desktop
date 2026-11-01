use std::path::Path;

use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

const LOCALTIME: &str = "/etc/localtime";
const ZONEINFO: &str = "zoneinfo/";

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
