use std::os::fd::BorrowedFd;
use std::sync::mpsc::Sender;

use bande_inbox::Inbox;
use bande_status::Update;
use bande_status::devices::{Control, follow_devices};
use bande_status::status::{Battery, Wifi};

pub struct Devices {
    inbox: Inbox<Update>,
    _controls: Sender<Control>,
}

impl Devices {
    pub fn follow(system: &zbus::blocking::Connection) -> Option<Self> {
        let inbox = Inbox::new()
            .inspect_err(|error| tracing::warn!("cannot follow the battery and Wi-Fi: {error}"))
            .ok()?;
        let controls = follow_devices(system, inbox.outbox());
        Some(Self {
            inbox,
            _controls: controls,
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inbox.fd()
    }

    pub fn latest(&mut self) -> Option<(Option<Battery>, Option<Wifi>)> {
        self.inbox
            .drain()
            .into_iter()
            .rev()
            .find_map(|update| match update {
                Update::Devices((battery, wifi, _)) => Some((battery, wifi)),
                _ => None,
            })
    }
}
