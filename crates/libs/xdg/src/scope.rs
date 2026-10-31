use std::fmt::Display;
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SYSTEMD: &str = "org.freedesktop.systemd1";
const MANAGER_PATH: &str = "/org/freedesktop/systemd1";
const MANAGER: &str = "org.freedesktop.systemd1.Manager";
const UNIT: &str = "org.freedesktop.systemd1.Unit";
const SCOPE: &str = "org.freedesktop.systemd1.Scope";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OOM_KILL: &str = "oom-kill";
const SETTLE: Duration = Duration::from_secs(2);
const SETTLE_STEP: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failed {
    Collect,
    Keep,
}

pub fn app_scope(app_id: &str, instance: impl Display) -> String {
    let mut escaped = String::with_capacity(app_id.len());
    for byte in app_id.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'.') {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("\\x{byte:02x}"));
        }
    }
    format!("app-unbound-{escaped}-{instance}.scope")
}

pub fn start_scope(bus: &Connection, unit: &str, pid: u32, failed: Failed) -> zbus::Result<()> {
    let mut properties = vec![
        ("PIDs", Value::from(vec![pid])),
        ("Slice", Value::from("app.slice")),
    ];
    if failed == Failed::Collect {
        properties.push(("CollectMode", Value::from("inactive-or-failed")));
    }
    bus.call_method(
        Some(SYSTEMD),
        MANAGER_PATH,
        Some(MANAGER),
        "StartTransientUnit",
        &(
            unit,
            "fail",
            properties,
            Vec::<(&str, Vec<(&str, Value)>)>::new(),
        ),
    )?;
    Ok(())
}

fn property(
    bus: &Connection,
    path: &OwnedObjectPath,
    interface: &str,
    name: &str,
) -> Option<String> {
    let reply = bus
        .call_method(
            Some(SYSTEMD),
            path,
            Some(PROPERTIES),
            "Get",
            &(interface, name),
        )
        .ok()?;
    let value: OwnedValue = reply.body().deserialize().ok()?;
    String::try_from(value).ok()
}

pub fn ran_out_of_memory(bus: &Connection, unit: &str) -> bool {
    let deadline = Instant::now() + SETTLE;
    loop {
        let Ok(reply) =
            bus.call_method(Some(SYSTEMD), MANAGER_PATH, Some(MANAGER), "GetUnit", &unit)
        else {
            return false;
        };
        let Ok(path) = reply.body().deserialize::<OwnedObjectPath>() else {
            return false;
        };
        match property(bus, &path, UNIT, "ActiveState").as_deref() {
            Some("failed") => {
                let result = property(bus, &path, SCOPE, "Result");
                if let Err(error) = bus.call_method(
                    Some(SYSTEMD),
                    MANAGER_PATH,
                    Some(MANAGER),
                    "ResetFailedUnit",
                    &unit,
                ) {
                    tracing::debug!("cannot clear {unit}: {error}");
                }
                return result.as_deref() == Some(OOM_KILL);
            }
            Some("active" | "activating" | "deactivating") if Instant::now() < deadline => {
                thread::sleep(SETTLE_STEP);
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use unbound_test_bus::SessionBus;
    use unbound_test_bus::systemd::FakeSystemd;

    use super::*;

    #[test]
    fn only_a_scope_that_failed_from_an_oom_kill_ran_out_of_memory() {
        let bus = SessionBus::start().unwrap();
        let systemd = FakeSystemd::start(&bus).unwrap();
        let client = bus.connect().unwrap();
        for unit in ["oom.scope", "timeout.scope", "ended.scope"] {
            start_scope(&client, unit, 1, Failed::Keep).unwrap();
        }
        systemd.fail("oom.scope", "oom-kill").unwrap();
        systemd.fail("timeout.scope", "timeout").unwrap();
        systemd.end("ended.scope").unwrap();

        assert!(ran_out_of_memory(&client, "oom.scope"));
        assert!(!ran_out_of_memory(&client, "timeout.scope"));
        assert!(!ran_out_of_memory(&client, "ended.scope"));
        assert!(!ran_out_of_memory(&client, "never.scope"));
        assert_eq!(systemd.reset(), ["oom.scope", "timeout.scope"]);
    }

    #[test]
    fn a_scope_still_stopping_is_waited_for() {
        let bus = SessionBus::start().unwrap();
        let systemd = FakeSystemd::start(&bus).unwrap();
        let client = bus.connect().unwrap();
        start_scope(&client, "slow.scope", 1, Failed::Keep).unwrap();
        let failing = thread::spawn(move || {
            thread::sleep(Duration::from_millis(200));
            systemd.fail("slow.scope", "oom-kill").unwrap();
            systemd
        });

        assert!(ran_out_of_memory(&client, "slow.scope"));
        failing.join().unwrap();
    }

    #[test]
    fn without_systemd_nothing_ran_out_of_memory() {
        let bus = SessionBus::start().unwrap();
        let client = bus.connect().unwrap();
        assert!(start_scope(&client, "lone.scope", 1, Failed::Keep).is_err());
        assert!(!ran_out_of_memory(&client, "lone.scope"));
    }
}
