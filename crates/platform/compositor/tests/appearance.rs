#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::mpsc;
use std::time::Duration;

use unbound_compositor::follow_reduced_motion;
use unbound_test_bus::SessionBus;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";
const TIMEOUT: Duration = Duration::from_secs(10);

struct Settings;

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_one(&self, namespace: String, key: String) -> zbus::fdo::Result<OwnedValue> {
        if namespace == APPEARANCE && key == "reduced-motion" {
            return OwnedValue::try_from(Value::from(1u32))
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()));
        }
        Err(zbus::fdo::Error::Failed("no such setting".to_owned()))
    }

    #[zbus(signal)]
    async fn setting_changed(
        emitter: &SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: Value<'_>,
    ) -> zbus::Result<()>;
}

#[test]
fn reduce_motion_follows_the_portals_reduced_motion_key() {
    let bus = SessionBus::start().expect("session bus");
    let portal = bus.connect().expect("portal connection");
    portal
        .object_server()
        .at(PORTAL_PATH, Settings)
        .expect("settings object");
    portal.request_name(PORTAL).expect("portal name");
    let (sender, readings) = mpsc::channel();

    follow_reduced_motion(
        bus.connect().expect("compositor connection"),
        move |reduce| sender.send(reduce).is_ok(),
    )
    .expect("follow the portal");
    let first = readings.recv_timeout(TIMEOUT).expect("first reading");
    let emitter = SignalEmitter::new(portal.inner(), PORTAL_PATH).expect("emitter");
    for (key, value) in [("color-scheme", 1u32), ("reduced-motion", 0u32)] {
        zbus::block_on(Settings::setting_changed(
            &emitter,
            APPEARANCE,
            key,
            Value::from(value),
        ))
        .expect("signal");
    }
    let changed = readings.recv_timeout(TIMEOUT).expect("change");

    assert!(first);
    assert!(!changed);
    assert!(readings.recv_timeout(Duration::from_millis(200)).is_err());
}
