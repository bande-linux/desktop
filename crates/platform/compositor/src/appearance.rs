use std::thread;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const REDUCED_MOTION: &str = "reduced-motion";

fn reduces(value: &OwnedValue) -> bool {
    u32::try_from(value).is_ok_and(|reduce| reduce != 0)
}

pub fn follow_reduced_motion(
    connection: Connection,
    mut publish: impl FnMut(bool) -> bool + Send + 'static,
) -> zbus::Result<()> {
    let settings = Proxy::new(&connection, PORTAL, PORTAL_PATH, SETTINGS)?;
    let changes = settings.receive_signal("SettingChanged")?;
    thread::spawn(move || {
        let first = settings
            .call::<_, _, OwnedValue>("ReadOne", &(NAMESPACE, REDUCED_MOTION))
            .is_ok_and(|value| reduces(&value));
        if !publish(first) {
            return;
        }
        for change in changes {
            let Ok((namespace, key, value)) =
                change.body().deserialize::<(String, String, OwnedValue)>()
            else {
                continue;
            };
            if namespace == NAMESPACE && key == REDUCED_MOTION && !publish(reduces(&value)) {
                return;
            }
        }
    });
    Ok(())
}
