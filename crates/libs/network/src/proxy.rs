use std::collections::HashMap;

use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

    fn deactivate_connection(&self, active: &ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn connectivity(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn connectivity_check_uri(&self) -> zbus::Result<String>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Device {
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn hw_address(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;

    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;

    #[zbus(property)]
    fn available_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.IP4Config",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Ip4Config {
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;

    #[zbus(property)]
    fn gateway(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn nameserver_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wired",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Wired {
    #[zbus(property)]
    fn carrier(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Wireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;

    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;

    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait SettingsConnection {
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, OwnedValue>>>;

    fn update(&self, properties: HashMap<String, HashMap<String, OwnedValue>>) -> zbus::Result<()>;

    fn delete(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait ActiveConnection {
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn connection(&self) -> zbus::Result<OwnedObjectPath>;

    #[zbus(signal, name = "StateChanged")]
    fn activation_state_changed(&self, state: u32, reason: u32) -> zbus::Result<()>;
}
