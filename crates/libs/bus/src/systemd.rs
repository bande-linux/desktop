use zbus::blocking::proxy::Builder;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, Value};

type ListedUnit = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

#[zbus::proxy(
    interface = "org.freedesktop.systemd1.Manager",
    default_service = "org.freedesktop.systemd1",
    default_path = "/org/freedesktop/systemd1"
)]
pub trait ServiceManager {
    fn get_unit(&self, name: &str) -> zbus::Result<OwnedObjectPath>;

    fn set_unit_properties(
        &self,
        name: &str,
        runtime: bool,
        properties: &[(&str, Value<'_>)],
    ) -> zbus::Result<()>;

    fn list_units_by_patterns(
        &self,
        states: &[&str],
        patterns: &[&str],
    ) -> zbus::Result<Vec<ListedUnit>>;

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

/// How much memory `unit` is guaranteed, in bytes.
pub fn memory_min(bus: &Connection, unit: &str) -> zbus::Result<u64> {
    let interface = match unit.ends_with(".slice") {
        true => "org.freedesktop.systemd1.Slice",
        false => "org.freedesktop.systemd1.Service",
    };
    let path = ServiceManagerProxyBlocking::new(bus)?.get_unit(unit)?;
    let unit: Proxy<'_> = Builder::new(bus)
        .destination("org.freedesktop.systemd1")?
        .path(path)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?;
    unit.get_property("MemoryMin")
}

/// Guarantees `unit` this much memory until the computer restarts or it is changed again.
pub fn set_memory_min(bus: &Connection, unit: &str, bytes: u64) -> zbus::Result<()> {
    ServiceManagerProxyBlocking::new(bus)?.set_unit_properties(
        unit,
        true,
        &[("MemoryMin", Value::from(bytes))],
    )
}
