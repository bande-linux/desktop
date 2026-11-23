use std::sync::mpsc::Receiver;
use std::thread;

use bande_datetime::MinuteTimer;
use bande_datetime::timedate::zone_changes;
use bande_inbox::Outbox;
use zbus::blocking::Connection;

use crate::Update;

pub fn follow_minutes(outbox: &Outbox<Update>, shown: &Receiver<()>) -> rustix::io::Result<()> {
    let timer = MinuteTimer::new()?;
    loop {
        timer.take()?;
        while shown.try_recv().is_ok() {}
        if !outbox.send(Update::Minute) || shown.recv().is_err() {
            return Ok(());
        }
        timer.arm()?;
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
