use std::collections::HashMap;

use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::{BUS_NAME, OBJECT_PATH, Urgency};

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
}

pub fn notify(connection: &Connection, request: &Request<'_>) -> zbus::Result<u32> {
    let actions: Vec<&str> = request
        .actions
        .iter()
        .flat_map(|(key, label)| [*key, *label])
        .collect();
    let urgency = Value::from(match request.urgency {
        Urgency::Low => 0u8,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    });
    let hints = HashMap::from([("urgency", urgency)]);
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
