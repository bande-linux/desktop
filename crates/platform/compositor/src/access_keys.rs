use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::thread;
use std::time::Duration;

use bande_config::{Accessibility, BUS_NAME, OBJECT_PATH};
use rustix::time::{
    Itimerspec, TimerfdClockId, TimerfdFlags, TimerfdTimerFlags, Timespec, timerfd_create,
    timerfd_settime,
};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::L10N;
use crate::state::{Compositor, State};

const SLOW_KEYS_DELAY: Duration = Duration::from_millis(300);
const BOUNCE_KEYS_DELAY: u32 = 300;
const SHIFT_PRESSES: usize = 5;
const SHIFT_PRESS_GAP: u32 = 600;
const CONFIRMATION_TIMEOUT: i32 = 20_000;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_RIGHTSHIFT: u32 = 54;
const MODIFIERS: [u32; 8] = [29, 42, 54, 56, 97, 100, 125, 126];

fn is_modifier(code: u32) -> bool {
    MODIFIERS.contains(&code)
}

fn is_shift(code: u32) -> bool {
    matches!(code, KEY_LEFTSHIFT | KEY_RIGHTSHIFT)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Latch {
    Latched,
    Locked,
}

pub(crate) type Delivery = (u32, bool, u32);

#[derive(Default)]
pub(crate) struct AccessKeys {
    sticky: bool,
    slow: bool,
    bounce: bool,
    latches: Vec<(u32, Latch)>,
    held_modifiers: Vec<(u32, bool)>,
    swallowed: Vec<u32>,
    released_at: HashMap<u32, u32>,
    shift_presses: Vec<u32>,
    slow_pending: Option<(u32, u32)>,
    timer: Option<OwnedFd>,
    last_time: u32,
}

impl AccessKeys {
    pub(crate) fn configure(&mut self, settings: &Accessibility) -> Vec<Delivery> {
        let mut releases = Vec::new();
        if !settings.sticky_keys {
            let time = self.last_time;
            releases.extend(self.latches.drain(..).map(|(code, _)| (code, false, time)));
        }
        if !settings.slow_keys {
            self.slow_pending = None;
        }
        self.sticky = settings.sticky_keys;
        self.slow = settings.slow_keys;
        self.bounce = settings.bounce_keys;
        releases
    }

    fn count_shift(&mut self, code: u32, pressed: bool, time: u32) -> bool {
        if !pressed {
            return false;
        }
        if !is_shift(code) {
            self.shift_presses.clear();
            return false;
        }
        if self
            .shift_presses
            .last()
            .is_some_and(|last| time.saturating_sub(*last) > SHIFT_PRESS_GAP)
        {
            self.shift_presses.clear();
        }
        self.shift_presses.push(time);
        if self.shift_presses.len() < SHIFT_PRESSES {
            return false;
        }
        self.shift_presses.clear();
        true
    }

    fn bounce(&mut self, code: u32, pressed: bool, time: u32) -> bool {
        if !self.bounce {
            return false;
        }
        if !pressed {
            self.released_at.insert(code, time);
            return false;
        }
        self.released_at
            .get(&code)
            .is_some_and(|released| time.saturating_sub(*released) < BOUNCE_KEYS_DELAY)
    }

    fn sticky(&mut self, code: u32, pressed: bool, time: u32) -> Vec<Delivery> {
        if !self.sticky {
            return vec![(code, pressed, time)];
        }
        if is_modifier(code) {
            return self.sticky_modifier(code, pressed, time);
        }
        if pressed {
            for (_, used) in &mut self.held_modifiers {
                *used = true;
            }
            return vec![(code, true, time)];
        }
        let mut deliveries = vec![(code, false, time)];
        self.latches.retain(|(modifier, latch)| {
            let keep = *latch == Latch::Locked;
            if !keep {
                deliveries.push((*modifier, false, time));
            }
            keep
        });
        deliveries
    }

    fn sticky_modifier(&mut self, code: u32, pressed: bool, time: u32) -> Vec<Delivery> {
        let latch = self
            .latches
            .iter()
            .position(|(modifier, _)| *modifier == code);
        if pressed {
            return match latch {
                Some(index) => {
                    self.swallowed.push(code);
                    match self.latches[index].1 {
                        Latch::Latched => {
                            self.latches[index].1 = Latch::Locked;
                            Vec::new()
                        }
                        Latch::Locked => {
                            self.latches.remove(index);
                            vec![(code, false, time)]
                        }
                    }
                }
                None => {
                    self.held_modifiers.push((code, false));
                    vec![(code, true, time)]
                }
            };
        }
        if let Some(index) = self.swallowed.iter().position(|held| *held == code) {
            self.swallowed.remove(index);
            return Vec::new();
        }
        let used = self
            .held_modifiers
            .iter()
            .position(|(modifier, _)| *modifier == code)
            .map(|index| self.held_modifiers.remove(index).1);
        if used == Some(false) {
            self.latches.push((code, Latch::Latched));
            return Vec::new();
        }
        vec![(code, false, time)]
    }
}

fn confirm_sticky_keys(session: Connection, turn_on: bool) {
    thread::spawn(move || {
        let (title, body, accept) = if turn_on {
            (
                L10N.tr("sticky-keys-turn-on-title"),
                L10N.tr("sticky-keys-turn-on-body"),
                L10N.tr("sticky-keys-turn-on"),
            )
        } else {
            (
                L10N.tr("sticky-keys-turn-off-title"),
                L10N.tr("sticky-keys-turn-off-body"),
                L10N.tr("sticky-keys-turn-off"),
            )
        };
        let cancel = L10N.tr("sticky-keys-keep");
        let Ok(notifications) = zbus::blocking::Proxy::new(
            &session,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        ) else {
            return;
        };
        let Ok(invoked) = notifications.receive_signal("ActionInvoked") else {
            return;
        };
        let hints: HashMap<&str, Value<'_>> = HashMap::from([("urgency", Value::from(2u8))]);
        let shown: zbus::Result<u32> = notifications.call(
            "Notify",
            &(
                L10N.tr("accessibility"),
                0u32,
                "preferences-desktop-accessibility",
                title,
                body,
                vec!["accept", accept.as_str(), "default", cancel.as_str()],
                hints,
                CONFIRMATION_TIMEOUT,
            ),
        );
        let Ok(id) = shown else {
            return;
        };
        for signal in invoked {
            let Ok((invoked_id, action)) = signal.body().deserialize::<(u32, String)>() else {
                continue;
            };
            if invoked_id != id {
                continue;
            }
            if action == "accept" {
                let set = session.call_method(
                    Some(BUS_NAME),
                    OBJECT_PATH,
                    Some("org.freedesktop.DBus.Properties"),
                    "Set",
                    &(BUS_NAME, "StickyKeys", Value::from(turn_on)),
                );
                if let Err(error) = set {
                    tracing::warn!("cannot change sticky keys: {error}");
                }
            }
            return;
        }
    });
}

impl State {
    pub(crate) fn access_keys(&mut self, code: u32, pressed: bool, time: u32) -> Vec<Delivery> {
        self.keys.last_time = time;
        if self.keys.count_shift(code, pressed, time)
            && let Some(session) = self.assistive.session_bus.clone()
        {
            confirm_sticky_keys(session, !self.keys.sticky);
        }
        if self.keys.bounce(code, pressed, time) {
            self.keys.swallowed.push(code);
            return Vec::new();
        }
        if pressed && self.keys.slow {
            if let Some((replaced, _)) = self.keys.slow_pending.replace((code, time)) {
                self.keys.swallowed.push(replaced);
            }
            self.arm_slow_keys();
            return Vec::new();
        }
        if !pressed
            && self
                .keys
                .slow_pending
                .is_some_and(|(pending, _)| pending == code)
        {
            self.keys.slow_pending = None;
            return Vec::new();
        }
        if !pressed
            && (!is_modifier(code) || !self.keys.sticky)
            && let Some(index) = self.keys.swallowed.iter().position(|held| *held == code)
        {
            self.keys.swallowed.remove(index);
            return Vec::new();
        }
        self.keys.sticky(code, pressed, time)
    }

    fn arm_slow_keys(&mut self) {
        if self.keys.timer.is_none() {
            let created = timerfd_create(
                TimerfdClockId::Monotonic,
                TimerfdFlags::NONBLOCK | TimerfdFlags::CLOEXEC,
            );
            let Ok(timer) = created else {
                return;
            };
            let Ok(watched) = timer.try_clone() else {
                return;
            };
            let inserted = self.blockers.insert_source(
                Generic::new(watched, Interest::READ, Mode::Level),
                |_, timer, state: &mut State| {
                    let mut expirations = [0u8; 8];
                    let _ = rustix::io::read(&*timer, &mut expirations);
                    state.accept_slow_key();
                    Ok(PostAction::Continue)
                },
            );
            if inserted.is_err() {
                return;
            }
            self.keys.timer = Some(timer);
        }
        let Some(timer) = &self.keys.timer else {
            return;
        };
        let delay = Itimerspec {
            it_interval: Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            },
            it_value: Timespec {
                tv_sec: 0,
                tv_nsec: i64::from(SLOW_KEYS_DELAY.subsec_nanos()),
            },
        };
        let _ = timerfd_settime(timer, TimerfdTimerFlags::empty(), &delay);
    }

    fn accept_slow_key(&mut self) {
        let Some((code, time)) = self.keys.slow_pending.take() else {
            return;
        };
        let accepted_at = time.wrapping_add(SLOW_KEYS_DELAY.as_millis() as u32);
        for (code, pressed, time) in self.keys.sticky(code, true, accepted_at) {
            self.deliver_key(code, pressed, time);
        }
        self.needs_redraw = true;
    }
}

impl State {
    pub(crate) fn configure_access_keys(&mut self, settings: &Accessibility) {
        for (code, pressed, time) in self.keys.configure(settings) {
            self.deliver_key(code, pressed, time);
        }
    }
}

impl Compositor {
    pub fn key(&mut self, evdev_code: u32, pressed: bool, time: u32) {
        self.state.note_activity();
        for (code, pressed, time) in self.state.access_keys(evdev_code, pressed, time) {
            self.state.deliver_key(code, pressed, time);
        }
    }
}
