use std::sync::mpsc::{self, Sender};

use bande_settings_panes::{SettingId, Value};
use bande_status::backlight::DisplayLight;
use zbus::blocking::Connection;

use crate::failure::failed;
use crate::follow::{Update, Updates, spawn};

fn level_update(level: f32) -> Update {
    Update::Values(vec![(SettingId::Brightness, Value::Fraction(level))])
}

pub fn follow_brightness(system: &Connection, updates: Updates) -> Option<Sender<f32>> {
    let light = DisplayLight::find()?;
    let follower = light.clone();
    let changes = updates.clone();
    spawn("brightness-follow", move || {
        if let Err(error) = follower.follow(|level| changes.send(level_update(level))) {
            tracing::warn!("cannot follow the brightness: {error}");
        }
    });
    let (sender, receiver) = mpsc::channel::<f32>();
    let system = system.clone();
    spawn("brightness", move || {
        while let Ok(first) = receiver.recv() {
            let level = receiver.try_iter().last().unwrap_or(first);
            if let Err(error) = light.set_level(&system, level) {
                updates.send(failed(SettingId::Brightness, &error, "service-brightness"));
                if let Some(level) = light.level() {
                    updates.send(level_update(level));
                }
            }
        }
    });
    Some(sender)
}
