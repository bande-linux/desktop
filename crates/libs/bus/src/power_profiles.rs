use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedValue;

pub const POWER_SAVER: &str = "power-saver";
pub const BALANCED: &str = "balanced";
pub const PERFORMANCE: &str = "performance";

#[zbus::proxy(
    interface = "org.freedesktop.UPower.PowerProfiles",
    default_service = "org.freedesktop.UPower.PowerProfiles",
    default_path = "/org/freedesktop/UPower/PowerProfiles"
)]
pub trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}

/// power-profiles-daemon, read afresh on every call rather than from a property cache.
pub fn power_profiles(system: &Connection) -> zbus::Result<PowerProfilesProxyBlocking<'static>> {
    PowerProfilesProxyBlocking::builder(system)
        .cache_properties(CacheProperties::No)
        .build()
}
