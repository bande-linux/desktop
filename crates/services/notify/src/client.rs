use std::collections::HashMap;

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::Value;
use zbus::{MatchRule, message};

use crate::{BUS_NAME, OBJECT_PATH, Urgency, WireApp, WirePresentation};

const INTERFACE: &str = "org.freedesktop.Notifications";

#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    pub app_name: &'a str,
    pub replaces_id: u32,
    pub app_icon: &'a str,
    pub summary: &'a str,
    pub body: &'a str,
    pub actions: &'a [(&'a str, &'a str)],
    pub urgency: Urgency,
    pub sound: Option<&'a str>,
    pub desktop_entry: Option<&'a str>,
}

pub fn notify(connection: &Connection, request: &Request<'_>) -> zbus::Result<u32> {
    let actions: Vec<&str> = request
        .actions
        .iter()
        .flat_map(|(key, label)| [*key, *label])
        .collect();
    let urgency = Value::from(crate::wire::urgency_code(request.urgency));
    let mut hints = HashMap::from([("urgency", urgency)]);
    if let Some(sound) = request.sound {
        hints.insert("sound-name", Value::from(sound));
    }
    if let Some(entry) = request.desktop_entry {
        hints.insert("desktop-entry", Value::from(entry));
    }
    connection
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some(INTERFACE),
            "Notify",
            &(
                request.app_name,
                request.replaces_id,
                request.app_icon,
                request.summary,
                request.body,
                actions,
                hints,
                -1i32,
            ),
        )?
        .body()
        .deserialize()
}

#[zbus::proxy(
    interface = "org.bande.Notifications",
    default_service = "org.bande.Notifications",
    default_path = "/org/freedesktop/Notifications",
    gen_async = false,
    blocking_name = "NotificationsProxy"
)]
pub trait Notifications {
    fn get_presentation(&self) -> zbus::Result<WirePresentation>;

    fn activate(&self, id: u32, action_key: &str, activation_token: &str) -> zbus::Result<bool>;

    fn dismiss(&self, id: u32) -> zbus::Result<()>;

    fn hide_banner(&self, id: u32) -> zbus::Result<()>;

    fn clear(&self, group: &str) -> zbus::Result<()>;

    fn clear_all(&self) -> zbus::Result<()>;

    fn hold_banners(&self, held: bool) -> zbus::Result<()>;

    fn quiet_while_connected(&self) -> zbus::Result<()>;

    fn quiet_for_full_screen(&self, full_screen: bool) -> zbus::Result<()>;

    fn get_apps(&self) -> zbus::Result<Vec<WireApp>>;

    fn set_app_rule(
        &self,
        group: &str,
        allow: bool,
        banners: &str,
        in_center: bool,
        sound: bool,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn do_not_disturb(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_do_not_disturb(&self, enabled: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    fn presentation_changed(&self, presentation: WirePresentation) -> zbus::Result<()>;
}

pub fn close(connection: &Connection, id: u32) -> zbus::Result<()> {
    connection.call_method(
        Some(BUS_NAME),
        OBJECT_PATH,
        Some(INTERFACE),
        "CloseNotification",
        &id,
    )?;
    Ok(())
}

/// What the notification server says about a notification it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    ActivationToken(u32, String),
    ActionInvoked(u32, String),
    Closed(u32),
}

/// Passes every notification signal on `connection` to `deliver` until `deliver` returns false.
/// Blocks, so it runs on a thread of its own.
pub fn listen(
    connection: &Connection,
    mut deliver: impl FnMut(Signal) -> bool,
) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(INTERFACE)?
        .path(OBJECT_PATH)?
        .build();
    for signal in MessageIterator::for_match_rule(rule, connection, None)?.flatten() {
        let header = signal.header();
        let body = signal.body();
        let signal = match header.member().map(|member| member.as_str()) {
            Some("ActivationToken") => body
                .deserialize::<(u32, String)>()
                .ok()
                .map(|(id, token)| Signal::ActivationToken(id, token)),
            Some("ActionInvoked") => body
                .deserialize::<(u32, String)>()
                .ok()
                .map(|(id, key)| Signal::ActionInvoked(id, key)),
            Some("NotificationClosed") => body
                .deserialize::<(u32, u32)>()
                .ok()
                .map(|(id, _)| Signal::Closed(id)),
            _ => None,
        };
        if let Some(signal) = signal
            && !deliver(signal)
        {
            break;
        }
    }
    Ok(())
}
