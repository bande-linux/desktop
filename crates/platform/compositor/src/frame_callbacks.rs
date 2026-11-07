use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_callback::WlCallback;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{SurfaceData, with_states};

use crate::pacing::ClientPace;
use crate::state::Compositor;

#[derive(Default)]
struct Pace(Mutex<ClientPace>);

fn with_pace<T>(states: &SurfaceData, use_pace: impl FnOnce(&mut ClientPace) -> T) -> Option<T> {
    states.data_map.insert_if_missing_threadsafe(Pace::default);
    let pace = states.data_map.get::<Pace>()?;
    let mut pace = pace.0.lock().ok()?;
    Some(use_pace(&mut pace))
}

pub(crate) fn note_commit(surface: &WlSurface) {
    with_states(surface, |states| {
        with_pace(states, |pace| pace.committed(Instant::now()));
    });
}

pub(crate) struct LatchedCallback {
    callback: WlCallback,
    surface: WlSurface,
    time_needed: Option<Duration>,
}

impl LatchedCallback {
    pub(crate) fn of(callback: WlCallback, surface: &WlSurface, states: &SurfaceData) -> Self {
        let now = Instant::now();
        let time_needed = with_pace(states, |pace| {
            pace.latched(now);
            pace.time_needed(now)
        })
        .flatten();
        Self {
            callback,
            surface: surface.clone(),
            time_needed,
        }
    }

    pub(crate) fn send(self, millis: u32) {
        send(self.callback, &self.surface, millis, false);
    }
}

pub(crate) struct DueCallback {
    at: Duration,
    delayed: bool,
    callback: WlCallback,
    surface: WlSurface,
}

impl DueCallback {
    pub(crate) fn send_now(self, millis: u32) {
        send(self.callback, &self.surface, millis, false);
    }
}

fn send(callback: WlCallback, surface: &WlSurface, millis: u32, delayed: bool) {
    callback.done(millis);
    if surface.is_alive() {
        with_states(surface, |states| {
            with_pace(states, |pace| pace.sent(Instant::now(), delayed));
        });
    }
}

fn millis_of(time: Duration) -> u32 {
    (time.as_millis() % (1 << 32)) as u32
}

impl Compositor {
    pub fn release_callbacks_on(&mut self, output: &str, now: Duration, next_start: Duration) {
        let Some(screen) = self.state.screen_named(output) else {
            return;
        };
        let screen = &mut self.state.screens[screen];
        let latched = std::mem::take(&mut screen.latched_frame_callbacks);
        screen
            .due_callbacks
            .extend(latched.into_iter().map(|latched| {
                let at = latched
                    .time_needed
                    .map(|needed| next_start.saturating_sub(needed))
                    .filter(|at| *at > now);
                DueCallback {
                    at: at.unwrap_or(now),
                    delayed: at.is_some(),
                    callback: latched.callback,
                    surface: latched.surface,
                }
            }));
    }

    pub fn send_due_callbacks_on(
        &mut self,
        output: &str,
        now: Duration,
    ) -> io::Result<Option<Duration>> {
        let Some(screen) = self.state.screen_named(output) else {
            return Ok(None);
        };
        let screen = &mut self.state.screens[screen];
        if screen.due_callbacks.is_empty() {
            return Ok(None);
        }
        let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut screen.due_callbacks)
            .into_iter()
            .partition(|callback| callback.at <= now);
        screen.due_callbacks = later;
        let next = screen
            .due_callbacks
            .iter()
            .map(|callback| callback.at)
            .min();
        if !due.is_empty() {
            for callback in due {
                send(
                    callback.callback,
                    &callback.surface,
                    millis_of(now),
                    callback.delayed,
                );
            }
            self.flush_clients()?;
        }
        Ok(next)
    }
}
