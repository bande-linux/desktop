use std::os::fd::OwnedFd;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use bande_l10n::tr;
use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;

use crate::adapter::ConfigProxyBlocking;
use crate::battery::UPowerProxyBlocking;
use crate::logind::{self, ManagerProxyBlocking};

const KEEP_RUNNING: &str = "keep-running";
const SETTLES_AFTER_WAKE: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    KeepRunning,
    SleepsHere,
    LeftToLogind,
}

impl Mode {
    pub fn of(keep_running_on_power: bool, hibernates: bool, configured_by_admin: bool) -> Self {
        match (keep_running_on_power, hibernates && !configured_by_admin) {
            (true, _) => Self::KeepRunning,
            (false, true) => Self::SleepsHere,
            (false, false) => Self::LeftToLogind,
        }
    }

    fn reason(self) -> String {
        match self {
            Self::KeepRunning => tr!("Keep running with the lid closed while on power"),
            Self::SleepsHere | Self::LeftToLogind => {
                tr!("Sleep with the lid closed and hibernate before the battery runs out")
            }
        }
    }
}

enum Event {
    Changed,
    Lid(bool),
    Woke { slept: bool },
    Settled(u64),
}

pub struct Lid {
    events: Sender<Event>,
}

impl Lid {
    pub fn woke(&self, slept: bool) {
        let _ = self.events.send(Event::Woke { slept });
    }
}

struct Watcher {
    system: Connection,
    manager: ManagerProxyBlocking<'static>,
    upower: UPowerProxyBlocking<'static>,
    config: ConfigProxyBlocking<'static>,
    configured_by_admin: bool,
    mode: Mode,
    held: Option<(Mode, OwnedFd)>,
    closed: bool,
    left_until_opened: bool,
    failed: bool,
    wakes: u64,
}

fn forward<T: Send + 'static>(
    changes: impl Iterator<Item = T> + Send + 'static,
    events: &Sender<Event>,
    event: impl Fn(T) -> Event + Send + 'static,
) {
    let events = events.clone();
    thread::spawn(move || {
        for change in changes {
            if events.send(event(change)).is_err() {
                return;
            }
        }
    });
}

pub fn follow(system: &Connection, session: &Connection) -> zbus::Result<Option<Lid>> {
    let upower = UPowerProxyBlocking::builder(system)
        .cache_properties(CacheProperties::No)
        .build()?;
    if !upower.lid_is_present()? {
        return Ok(None);
    }
    let config = ConfigProxyBlocking::new(session)?;
    let manager = ManagerProxyBlocking::builder(system)
        .cache_properties(CacheProperties::No)
        .build()?;
    let (events, received) = mpsc::channel();
    let watched = UPowerProxyBlocking::new(system)?;
    forward(watched.receive_on_battery_changed(), &events, |_| {
        Event::Changed
    });
    forward(watched.receive_lid_is_closed_changed(), &events, |change| {
        change.get().map_or(Event::Changed, Event::Lid)
    });
    forward(config.receive_lid_on_power_changed(), &events, |_| {
        Event::Changed
    });
    let watcher = Watcher {
        system: system.clone(),
        closed: upower.lid_is_closed()?,
        manager,
        upower,
        config,
        configured_by_admin: logind::logind_sets("HandleLidSwitch"),
        mode: Mode::LeftToLogind,
        held: None,
        left_until_opened: false,
        failed: false,
        wakes: 0,
    };
    let timer = events.clone();
    thread::spawn(move || watcher.run(&received, &timer));
    Ok(Some(Lid { events }))
}

impl Watcher {
    fn run(mut self, events: &Receiver<Event>, timer: &Sender<Event>) {
        self.follow();
        for event in events {
            match event {
                Event::Changed => self.follow(),
                Event::Lid(closed) => self.set_closed(closed),
                Event::Woke { slept } => {
                    self.wakes += 1;
                    self.failed |= !slept && self.closed;
                    let (wake, timer) = (self.wakes, timer.clone());
                    thread::spawn(move || {
                        thread::sleep(SETTLES_AFTER_WAKE);
                        let _ = timer.send(Event::Settled(wake));
                    });
                }
                Event::Settled(wake) if wake == self.wakes => match self.manager.lid_closed() {
                    Ok(closed) => {
                        self.closed = closed;
                        if closed {
                            self.sleep_if_closed();
                        } else {
                            self.opened();
                        }
                    }
                    Err(error) => tracing::warn!("cannot tell whether the lid is closed: {error}"),
                },
                Event::Settled(_) => {}
            }
        }
    }

    fn follow(&mut self) {
        let on_battery = self.upower.on_battery().unwrap_or(false);
        let keep_running = !on_battery
            && self
                .config
                .lid_on_power()
                .is_ok_and(|action| action == KEEP_RUNNING);
        let hibernates = logind::hibernates_after_sleep(&self.manager)
            .inspect_err(|error| tracing::warn!("cannot ask whether sleep may hibernate: {error}"))
            .unwrap_or(false);
        let mode = Mode::of(keep_running, hibernates, self.configured_by_admin);
        let became_sleeper = mode == Mode::SleepsHere && self.mode != mode;
        self.mode = mode;
        self.hold();
        if became_sleeper && self.closed {
            self.sleep_if_closed();
        }
    }

    fn hold(&mut self) {
        let wanted = match self.mode {
            Mode::KeepRunning => Some(Mode::KeepRunning),
            Mode::SleepsHere if !self.left_until_opened => Some(Mode::SleepsHere),
            Mode::SleepsHere | Mode::LeftToLogind => None,
        };
        if self.held.as_ref().map(|(mode, _)| *mode) == wanted {
            return;
        }
        self.held = wanted.and_then(|mode| {
            logind::inhibit(
                &self.system,
                "handle-lid-switch",
                &tr!("Power"),
                &mode.reason(),
                "block",
            )
            .inspect_err(|error| tracing::warn!("cannot handle the lid: {error}"))
            .ok()
            .map(|fd| (mode, fd))
        });
    }

    fn set_closed(&mut self, closed: bool) {
        if closed == self.closed {
            return;
        }
        self.closed = closed;
        match closed {
            true => self.sleep_if_closed(),
            false => self.opened(),
        }
    }

    fn opened(&mut self) {
        self.left_until_opened = false;
        self.failed = false;
        self.hold();
    }

    fn leave_to_logind(&mut self) {
        self.left_until_opened = true;
        self.hold();
    }

    fn sleep_if_closed(&mut self) {
        if self.mode != Mode::SleepsHere || self.left_until_opened || self.failed {
            return;
        }
        let docked = self.manager.docked().unwrap_or_else(|error| {
            tracing::warn!("cannot tell whether a display is connected: {error}");
            true
        });
        let blocked = self
            .manager
            .block_inhibited()
            .is_ok_and(|what| what.split(':').any(|what| what == "sleep"));
        if docked || blocked {
            self.leave_to_logind();
            return;
        }
        match logind::sleep(&self.manager, false) {
            Ok(true) => {}
            Ok(false) => {
                tracing::warn!("logind does not let the computer sleep with the lid closed");
                self.leave_to_logind();
            }
            Err(error) => {
                tracing::warn!("cannot sleep with the lid closed: {error}");
                self.leave_to_logind();
            }
        }
    }
}
