use std::io::{self, Read};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, poll};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use crate::controller::IdleController;

struct Watcher {
    controller: Arc<IdleController>,
    seat: Option<WlSeat>,
    notifier: Option<ExtIdleNotifierV1>,
    shortest: Option<Duration>,
}

#[derive(Debug)]
pub enum WatchError {
    Wayland(String),
    Missing(&'static str),
    Io(io::Error),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wayland(error) => write!(f, "Wayland: {error}"),
            Self::Missing(global) => write!(f, "the compositor does not offer {global}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WatchError {}

fn wayland(error: impl std::fmt::Display) -> WatchError {
    WatchError::Wayland(error.to_string())
}

impl Watcher {
    fn watch(&mut self, queue: &EventQueue<Self>) -> Vec<ExtIdleNotificationV1> {
        let (Some(notifier), Some(seat)) = (&self.notifier, &self.seat) else {
            return Vec::new();
        };
        let thresholds = self.controller.thresholds();
        self.shortest = thresholds.iter().min().copied();
        thresholds
            .into_iter()
            .map(|threshold| {
                let millis = u32::try_from(threshold.as_millis()).unwrap_or(u32::MAX);
                notifier.get_idle_notification(millis, seat, &queue.handle(), threshold)
            })
            .collect()
    }
}

pub fn watch_idle(
    connection: Connection,
    controller: Arc<IdleController>,
) -> Result<JoinHandle<Result<(), WatchError>>, WatchError> {
    let mut timer_restarts = controller.idle_timer_restarts().map_err(WatchError::Io)?;
    let mut queue = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut watcher = Watcher {
        controller,
        seat: None,
        notifier: None,
        shortest: None,
    };
    queue.roundtrip(&mut watcher).map_err(wayland)?;
    watcher
        .notifier
        .as_ref()
        .ok_or(WatchError::Missing("ext_idle_notifier_v1"))?;
    watcher
        .seat
        .as_ref()
        .ok_or(WatchError::Missing("wl_seat"))?;
    Ok(thread::spawn(move || {
        let mut notifications = watcher.watch(&queue);
        loop {
            queue.dispatch_pending(&mut watcher).map_err(wayland)?;
            queue.flush().map_err(wayland)?;
            let Some(guard) = queue.prepare_read() else {
                continue;
            };
            let (wayland_ready, timers_restarted) = {
                let connection_fd = guard.connection_fd();
                let changes_fd = timer_restarts.as_fd();
                let mut fds = [
                    PollFd::new(&connection_fd, PollFlags::IN),
                    PollFd::new(&changes_fd, PollFlags::IN),
                ];
                match poll(&mut fds, None) {
                    Ok(_) | Err(rustix::io::Errno::INTR) => {}
                    Err(error) => return Err(WatchError::Io(error.into())),
                }
                (!fds[0].revents().is_empty(), !fds[1].revents().is_empty())
            };
            if wayland_ready {
                match guard.read() {
                    Ok(_) => {}
                    Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(wayland(error)),
                }
            } else {
                drop(guard);
            }
            if timers_restarted {
                let mut drained = [0u8; 64];
                if timer_restarts.read(&mut drained).map_err(WatchError::Io)? == 0 {
                    return Ok(());
                }
                for notification in notifications.drain(..) {
                    notification.destroy();
                }
                notifications = watcher.watch(&queue);
            }
        }
    }))
}

impl Dispatch<WlRegistry, ()> for Watcher {
    fn event(
        watcher: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match interface.as_str() {
                "wl_seat" if watcher.seat.is_none() => {
                    watcher.seat = Some(registry.bind(name, 1, handle, ()));
                }
                "ext_idle_notifier_v1" => {
                    watcher.notifier = Some(registry.bind(name, 1, handle, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<ExtIdleNotificationV1, Duration> for Watcher {
    fn event(
        watcher: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        threshold: &Duration,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => watcher.controller.idle_for(*threshold),
            ext_idle_notification_v1::Event::Resumed if watcher.shortest == Some(*threshold) => {
                watcher.controller.resumed();
            }
            _ => {}
        }
    }
}

delegate_noop!(Watcher: ignore WlSeat);
delegate_noop!(Watcher: ExtIdleNotifierV1);
