use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait ServiceManager {
    fn get_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;

    #[zbus(property)]
    fn virtualization(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Service",
    default_service = "org.freedesktop.systemd1"
)]
trait Service {
    #[zbus(property, name = "MainPID")]
    fn main_pid(&self) -> zbus::Result<u32>;
}

/// The main process of a loaded service of the service manager on `bus`, or `None` while it
/// runs none.
pub fn main_pid(bus: &Connection, unit: &str) -> zbus::Result<Option<u32>> {
    let path = ServiceManagerProxyBlocking::new(bus)?.get_unit(unit)?;
    let pid = ServiceProxyBlocking::builder(bus)
        .path(path)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?
        .main_pid()?;
    Ok((pid != 0).then_some(pid))
}
