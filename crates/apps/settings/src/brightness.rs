use std::sync::mpsc::Sender;

use bande_settings_panes::SettingId;
use bande_status::brightness::{LevelRequest, follow_levels, level_requests};
use zbus::blocking::Connection;

use crate::failure::failed;
use crate::follow::{Update, Updates};

pub fn follow_brightness(session: &Connection, updates: Updates) -> Option<Sender<LevelRequest>> {
    let changes = updates.clone();
    if let Err(error) = follow_levels(session, move |levels| {
        changes.send(Update::DisplayLevels(levels))
    }) {
        tracing::warn!("cannot follow the brightness: {error}");
    }
    level_requests(session, move |error| {
        updates.send(failed(SettingId::Brightness, &error, "service-brightness"));
    })
    .inspect_err(|error| tracing::warn!("cannot set the brightness: {error}"))
    .ok()
}
