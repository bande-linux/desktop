use std::thread;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const REDUCED_MOTION: &str = "reduced-motion";
const COLOR_SCHEME: &str = "color-scheme";
const PREFER_DARK: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SystemAppearance {
    pub reduce_motion: bool,
    pub dark: bool,
}

impl SystemAppearance {
    fn apply(&mut self, key: &str, value: &OwnedValue) {
        let value = u32::try_from(value).ok();
        match key {
            REDUCED_MOTION => self.reduce_motion = value.is_some_and(|reduce| reduce != 0),
            COLOR_SCHEME => self.dark = value == Some(PREFER_DARK),
            _ => {}
        }
    }
}

pub fn follow_appearance(
    connection: Connection,
    mut publish: impl FnMut(SystemAppearance) -> bool + Send + 'static,
) -> zbus::Result<()> {
    let settings = Proxy::new(&connection, PORTAL, PORTAL_PATH, SETTINGS)?;
    let changes = settings.receive_signal("SettingChanged")?;
    thread::spawn(move || {
        let mut appearance = SystemAppearance::default();
        for key in [REDUCED_MOTION, COLOR_SCHEME] {
            if let Ok(value) = settings.call::<_, _, OwnedValue>("ReadOne", &(NAMESPACE, key)) {
                appearance.apply(key, &value);
            }
        }
        if !publish(appearance) {
            return;
        }
        for change in changes {
            let Ok((namespace, key, value)) =
                change.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if namespace != NAMESPACE {
                continue;
            }
            let before = appearance;
            appearance.apply(&key, &value);
            if appearance != before && !publish(appearance) {
                return;
            }
        }
    });
    Ok(())
}
