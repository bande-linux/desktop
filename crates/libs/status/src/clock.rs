use std::thread;

use unbound_datetime::MinuteTimer;
use unbound_datetime::timedate::zone_changes;
use unbound_inbox::Outbox;
use zbus::blocking::Connection;

use crate::Update;

pub fn follow_minutes(outbox: &Outbox<Update>) -> rustix::io::Result<()> {
    let timer = MinuteTimer::new()?;
    loop {
        timer.wait()?;
        if !outbox.send(Update::Minute) {
            return Ok(());
        }
    }
}

pub fn follow_zone(system: &Connection, outbox: Outbox<Update>) -> zbus::Result<()> {
    let zones = zone_changes(system)?;
    thread::spawn(move || {
        for _ in zones {
            if !outbox.send(Update::Minute) {
                return;
            }
        }
    });
    Ok(())
}
