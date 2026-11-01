pub mod capture;
pub mod dialog_process;
pub mod file_chooser;
pub mod gtk;
pub mod request;
pub mod screencast;
pub mod screenshot;
pub mod settings;
pub mod wallpaper;

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.bande";
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
pub const FRONTEND: &str = "org.freedesktop.portal.Desktop";

pub async fn from_frontend(
    connection: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> bool {
    let Ok(bus) = zbus::fdo::DBusProxy::new(connection).await else {
        return false;
    };
    let Ok(owner) = bus
        .get_name_owner(zbus::names::BusName::from_static_str_unchecked(FRONTEND))
        .await
    else {
        return false;
    };
    header.sender().map(|sender| sender.as_str()) == Some(owner.as_str())
}
