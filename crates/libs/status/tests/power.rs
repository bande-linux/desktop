#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use unbound_status::devices::{PowerAction, powers, use_power};
use unbound_test_bus::SessionBus;

type Calls = Arc<Mutex<Vec<(&'static str, bool)>>>;

struct FakeLogin {
    calls: Calls,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeLogin {
    fn can_suspend(&self) -> &str {
        "na"
    }

    fn can_reboot(&self) -> &str {
        "yes"
    }

    fn can_power_off(&self) -> &str {
        "challenge"
    }

    fn reboot(&self, interactive: bool) {
        self.calls.lock().unwrap().push(("reboot", interactive));
    }
}

#[test]
fn only_what_logind_allows_without_asking_is_offered_and_used_without_a_prompt() {
    let bus = SessionBus::start().unwrap();
    let calls = Calls::default();
    let _logind = zbus::blocking::connection::Builder::address(bus.address())
        .unwrap()
        .name("org.freedesktop.login1")
        .unwrap()
        .serve_at(
            "/org/freedesktop/login1",
            FakeLogin {
                calls: Arc::clone(&calls),
            },
        )
        .unwrap()
        .build()
        .unwrap();
    let system = bus.connect().unwrap();

    let offered = powers(&system).unwrap();
    use_power(&system, PowerAction::Restart).unwrap();

    assert_eq!(offered, [PowerAction::Restart]);
    assert_eq!(*calls.lock().unwrap(), [("reboot", false)]);
}
