use std::sync::mpsc;
use std::time::Duration;

use unbound_compositor::{SystemAppearance, follow_appearance};
use unbound_graphics::TextSize;
use unbound_test_bus::SessionBus;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const APPEARANCE: &str = "org.freedesktop.appearance";
const UNBOUND_APPEARANCE: &str = "org.unbound.appearance";
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
fn reduce_motion_dark_mode_and_text_size_follow_the_portals_appearance_keys() {
    let bus = SessionBus::start().expect("session bus");
    let portal = bus.connect().expect("portal connection");
    portal
        .object_server()
        .at(PORTAL_PATH, Settings)
        .expect("settings object");
    portal.request_name(PORTAL).expect("portal name");
    let (sender, readings) = mpsc::channel();

    follow_appearance(bus.connect().expect("compositor connection"), move |look| {
        sender.send(look).is_ok()
    })
    .expect("follow the portal");
    let first = readings.recv_timeout(TIMEOUT).expect("first reading");
    let emitter = SignalEmitter::new(portal.inner(), PORTAL_PATH).expect("emitter");
    for (namespace, key, value) in [
        (APPEARANCE, "color-scheme", 1u32),
        (APPEARANCE, "reduced-motion", 0u32),
        (UNBOUND_APPEARANCE, "text-size", 150u32),
    ] {
        zbus::block_on(Settings::setting_changed(
            &emitter,
            namespace,
            key,
            Value::from(value),
        ))
        .expect("signal");
    }
    let dark = readings.recv_timeout(TIMEOUT).expect("dark");
    let calm = readings.recv_timeout(TIMEOUT).expect("motion");
    let larger = readings.recv_timeout(TIMEOUT).expect("text size");

    let regular = TextSize::DEFAULT;
    assert_eq!(
        [first, dark, calm, larger],
        [
            SystemAppearance {
                reduce_motion: true,
                dark: false,
                text_size: regular,
            },
            SystemAppearance {
                reduce_motion: true,
                dark: true,
                text_size: regular,
            },
            SystemAppearance {
                reduce_motion: false,
                dark: true,
                text_size: regular,
            },
            SystemAppearance {
                reduce_motion: false,
                dark: true,
                text_size: TextSize::from_percent(150),
            },
        ]
    );
    assert!(readings.recv_timeout(Duration::from_millis(200)).is_err());
}
