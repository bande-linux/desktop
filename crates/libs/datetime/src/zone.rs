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

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn point(link: &Path, target: &str) -> std::io::Result<()> {
        let _ = std::fs::remove_file(link);
        symlink(target, link)
    }

    #[test]
    fn a_repointed_localtime_is_read_on_the_very_next_call() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        let link = directory.path().join("localtime");
        point(&link, "../usr/share/zoneinfo/Asia/Tokyo")?;
        assert_eq!(zone_at(&link).iana_name(), Some("Asia/Tokyo"));
        point(&link, "/usr/share/zoneinfo/America/New_York")?;
        assert_eq!(zone_at(&link).iana_name(), Some("America/New_York"));
        Ok(())
    }

    #[test]
    fn a_three_part_zone_keeps_its_whole_name() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        let link = directory.path().join("localtime");
        point(
            &link,
            "../usr/share/zoneinfo/America/Argentina/Buenos_Aires",
        )?;
        assert_eq!(
            zone_at(&link).iana_name(),
            Some("America/Argentina/Buenos_Aires")
        );
        Ok(())
    }

    #[test]
    fn a_link_outside_zoneinfo_falls_back_to_the_system_zone() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        let link = directory.path().join("localtime");
        point(&link, "/nowhere/Mars/Olympus")?;
        assert_eq!(zone_at(&link), TimeZone::system());
        Ok(())
    }
}
