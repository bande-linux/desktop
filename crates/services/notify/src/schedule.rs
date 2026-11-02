use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use bande_datetime::ClockChanges;
use bande_datetime::daylight::within_daily;
use jiff::civil::Time;
use rustix::event::{EventfdFlags, PollFd, PollFlags, eventfd, poll};
use rustix::io::{Errno, read, write};
use zbus::blocking::Connection;

type Window = Option<(Time, Time)>;

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config",
    gen_async = false,
    blocking_name = "ConfigProxy"
)]
trait Config {
    #[zbus(property)]
    fn focus_schedule(&self) -> zbus::Result<(bool, String, String)>;

    #[zbus(property)]
    fn show_previews(&self) -> zbus::Result<bool>;
}

fn time(text: &str) -> Option<Time> {
    let (hour, minute) = text.split_once(':')?;
    Time::new(hour.parse().ok()?, minute.parse().ok()?, 0, 0).ok()
}

fn window((on, from, to): (bool, String, String)) -> Window {
    on.then(|| Some((time(&from)?, time(&to)?))).flatten()
}

fn nudge(nudges: &OwnedFd) {
    let _ = write(nudges, &1u64.to_ne_bytes());
}

pub(crate) fn follow_schedule(
    session: &Connection,
    apply: impl Fn(bool) + Send + 'static,
) -> std::io::Result<()> {
    let config = ConfigProxy::new(session).map_err(std::io::Error::other)?;
    let clock = ClockChanges::new()?;
    let nudges = Arc::new(eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?);
    let current: Arc<Mutex<Window>> =
        Arc::new(Mutex::new(config.focus_schedule().ok().and_then(window)));
    let watched = Arc::clone(&current);
    let woken = Arc::clone(&nudges);
    thread::spawn(move || {
        for change in config.receive_focus_schedule_changed() {
            let schedule = change.get().ok().and_then(window);
            *watched.lock().unwrap_or_else(PoisonError::into_inner) = schedule;
            nudge(&woken);
        }
    });
    thread::spawn(move || {
        let mut applied = None;
        loop {
            let schedule = *current.lock().unwrap_or_else(PoisonError::into_inner);
            let state =
                schedule.and_then(|(from, to)| within_daily(&bande_datetime::now(), from, to));
            let next = match state {
                Some((inside, next)) => {
                    if applied != Some(inside) {
                        apply(inside);
                        applied = Some(inside);
                    }
                    Some(next.as_second())
                }
                None => {
                    applied = None;
                    None
                }
            };
            if let Err(error) = clock.wake_at(next) {
                tracing::warn!("cannot wait for the Do Not Disturb schedule: {error}");
                return;
            }
            let mut fds = [
                PollFd::new(&clock, PollFlags::IN),
                PollFd::new(&*nudges, PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) | Err(Errno::INTR) => {}
                Err(error) => {
                    tracing::warn!("cannot wait for the Do Not Disturb schedule: {error}");
                    return;
                }
            }
            let _ = clock.take();
            let mut count = [0; 8];
            let _ = read(&*nudges, &mut count);
        }
    });
    Ok(())
}

pub(crate) fn follow_previews(
    session: &Connection,
    apply: impl Fn(bool) + Send + 'static,
) -> zbus::Result<()> {
    let config = ConfigProxy::new(session)?;
    thread::spawn(move || {
        apply(config.show_previews().unwrap_or(true));
        for change in config.receive_show_previews_changed() {
            if let Ok(shown) = change.get() {
                apply(shown);
            }
        }
    });
    Ok(())
}
