use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedValue;

pub const APPEARANCE: &str = "org.freedesktop.appearance";
pub const UNBOUND_APPEARANCE: &str = "org.unbound.appearance";
pub const PREFER_DARK: u32 = 1;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";

pub fn follow<S: Copy + PartialEq>(
    connection: &Connection,
    namespaces: &[&str],
    mut state: S,
    apply: impl Fn(&mut S, &str, &str, OwnedValue),
    mut publish: impl FnMut(S) -> bool,
) -> zbus::Result<()> {
    let settings = Proxy::new(connection, PORTAL, PORTAL_PATH, SETTINGS)?;
    let changes = settings.receive_signal("SettingChanged")?;
    if let Ok(all) = settings
        .call::<_, _, HashMap<String, HashMap<String, OwnedValue>>>("ReadAll", &(namespaces,))
    {
        for (namespace, values) in all {
            for (key, value) in values {
                apply(&mut state, &namespace, &key, value);
            }
        }
    }
    if !publish(state) {
        return Ok(());
    }
    for change in changes {
        let Ok((namespace, key, value)) =
            change.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        let before = state;
        apply(&mut state, &namespace, &key, value);
        if state != before && !publish(state) {
            break;
        }
    }
    Ok(())
}
