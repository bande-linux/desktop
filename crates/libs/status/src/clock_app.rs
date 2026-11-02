use std::thread;

use bande_inbox::Outbox;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

use crate::Update;

pub const CLOCK_APP: &str = "org.bande.Clock";
const CLOCK_PATH: &str = "/org/bande/Clock";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClockApp {
    pub timers: Vec<(String, i64)>,
    pub next_alarm: Option<(String, i64)>,
}

fn property(session: &Connection, name: &str) -> zbus::Result<OwnedValue> {
    session
        .call_method(
            Some(CLOCK_APP),
            CLOCK_PATH,
            Some(PROPERTIES),
            "Get",
            &(CLOCK_APP, name),
        )?
        .body()
        .deserialize()
}

fn read(session: &Connection) -> ClockApp {
    let timers = property(session, "Timers")
        .and_then(|value| Ok(Vec::<(String, i64)>::try_from(value)?))
        .unwrap_or_default();
    let next_alarm = property(session, "NextAlarm")
        .and_then(|value| Ok(<(String, i64)>::try_from(value)?))
        .ok()
        .filter(|(_, at)| *at > 0);
    ClockApp { timers, next_alarm }
}

fn changes(session: &Connection, rule: MatchRule<'static>) -> zbus::Result<MessageIterator> {
    MessageIterator::for_match_rule(rule, session, None)
}

pub fn follow_clock_app(session: &Connection, outbox: &Outbox<Update>) -> zbus::Result<()> {
    let owners = changes(
        session,
        MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .add_arg(CLOCK_APP)?
            .build(),
    )?;
    let properties = changes(
        session,
        MatchRule::builder()
            .msg_type(message::Type::Signal)
            .interface(PROPERTIES)?
            .member("PropertiesChanged")?
            .path(CLOCK_PATH)?
            .add_arg(CLOCK_APP)?
            .build(),
    )?;
    let (watching, relay) = (session.clone(), outbox.clone());
    thread::spawn(move || {
        for _ in owners {
            if !relay.send(Update::ClockApp(read(&watching))) {
                break;
            }
        }
    });
    if !outbox.send(Update::ClockApp(read(session))) {
        return Ok(());
    }
    for _ in properties {
        if !outbox.send(Update::ClockApp(read(session))) {
            break;
        }
    }
    Ok(())
}
