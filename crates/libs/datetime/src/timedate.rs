use zbus::blocking::Connection;
use zbus::blocking::fdo::{PropertiesChangedIterator, PropertiesProxy};

const TIMEDATE: &str = "org.freedesktop.timedate1";
const TIMEDATE_PATH: &str = "/org/freedesktop/timedate1";

#[zbus::proxy(
    interface = "org.freedesktop.timedate1",
    default_service = "org.freedesktop.timedate1",
    default_path = "/org/freedesktop/timedate1"
)]
pub trait Timedate {
    #[zbus(property)]
    fn timezone(&self) -> zbus::Result<String>;
    #[zbus(property, name = "NTP")]
    fn ntp(&self) -> zbus::Result<bool>;
    #[zbus(property, name = "CanNTP")]
    fn can_ntp(&self) -> zbus::Result<bool>;
    fn list_timezones(&self) -> zbus::Result<Vec<String>>;
    fn set_timezone(&self, zone: &str, interactive: bool) -> zbus::Result<()>;
    #[zbus(name = "SetNTP")]
    fn set_ntp(&self, on: bool, interactive: bool) -> zbus::Result<()>;
    fn set_time(&self, microseconds: i64, relative: bool, interactive: bool) -> zbus::Result<()>;
}

pub fn timedate_changes(system: &Connection) -> zbus::Result<PropertiesChangedIterator> {
    PropertiesProxy::builder(system)
        .destination(TIMEDATE)?
        .path(TIMEDATE_PATH)?
        .build()?
        .receive_properties_changed()
}

pub fn zone_changes(system: &Connection) -> zbus::Result<impl Iterator<Item = String> + use<>> {
    Ok(timedate_changes(system)?.filter_map(|signal| {
        let arguments = signal.args().ok()?;
        let zone = arguments.changed_properties().get("Timezone")?;
        <&str>::try_from(zone).ok().map(str::to_owned)
    }))
}
