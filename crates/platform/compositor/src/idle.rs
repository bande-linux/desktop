use std::time::Duration;

use smithay::reexports::wayland_protocols::ext::idle_notify::v1::server::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use smithay::reexports::wayland_protocols::ext::idle_notify::v1::server::ext_idle_notifier_v1::{
    self, ExtIdleNotifierV1,
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::compositor::get_parent;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;

use crate::state::{Compositor, State};

const NOTIFIER_VERSION: u32 = 2;

pub(crate) struct Watch {
    notification: ExtIdleNotificationV1,
    timeout: Duration,
    obeys_inhibitors: bool,
    idled: bool,
    since: Option<Duration>,
}

#[derive(Default)]
pub(crate) struct Idle {
    watches: Vec<Watch>,
    inhibitors: Vec<WlSurface>,
    last_activity: Option<Duration>,
    activity: bool,
    inhibited: bool,
    inhibition_ended: Duration,
}

pub(crate) fn create_global(display: &DisplayHandle) {
    display.create_global::<State, ExtIdleNotifierV1, ()>(NOTIFIER_VERSION, ());
}

impl State {
    pub(crate) fn note_activity(&mut self) {
        self.idle.activity = true;
        self.watchdog.note_interaction();
        self.power_on_every_output();
    }

    fn root_of_surface(surface: &WlSurface) -> WlSurface {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        root
    }

    fn inhibited(&self) -> bool {
        if self.idle.inhibitors.is_empty() {
            return false;
        }
        let shown: Vec<_> = (0..self.screens.len())
            .flat_map(|screen| self.stack(screen))
            .collect();
        self.idle.inhibitors.iter().any(|surface| {
            let root = Self::root_of_surface(surface);
            surface.is_alive() && shown.iter().any(|(visible, _)| *visible == root)
        })
    }
}

impl Compositor {
    pub fn time_changed(&mut self) {
        self.state.night_light.recheck();
    }

    pub fn woke(&mut self) {
        self.state.night_light.recheck();
        self.state.note_activity();
        self.state.needs_redraw = true;
    }

    pub fn idle_tick(&mut self, now: Duration) -> Option<Duration> {
        let state = &mut self.state;
        state
            .idle
            .watches
            .retain(|watch| watch.notification.is_alive());
        state.idle.inhibitors.retain(Resource::is_alive);
        let mut told = false;
        if std::mem::take(&mut state.idle.activity) || state.idle.last_activity.is_none() {
            state.idle.last_activity = Some(now);
            for watch in state.idle.watches.iter_mut().filter(|watch| watch.idled) {
                watch.notification.resumed();
                watch.idled = false;
                told = true;
            }
        }
        if state
            .idle
            .watches
            .iter()
            .any(|watch| !watch.idled && watch.obeys_inhibitors)
        {
            let inhibited = state.inhibited();
            if state.idle.inhibited && !inhibited {
                state.idle.inhibition_ended = now;
            }
            state.idle.inhibited = inhibited;
        }
        let inhibited = state.idle.inhibited;
        let last = state.idle.last_activity.unwrap_or(now);
        let inhibition_ended = state.idle.inhibition_ended;
        let mut next = None::<Duration>;
        for watch in state.idle.watches.iter_mut().filter(|watch| !watch.idled) {
            let since = *watch.since.get_or_insert(now);
            let mut start = last.max(since);
            if watch.obeys_inhibitors {
                if inhibited {
                    continue;
                }
                start = start.max(inhibition_ended);
            }
            let due = start + watch.timeout;
            if now >= due {
                watch.notification.idled();
                watch.idled = true;
                told = true;
            } else {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        if told && let Err(error) = self.flush() {
            tracing::warn!("cannot tell the idle watchers: {error}");
        }
        next
    }
}

impl GlobalDispatch<ExtIdleNotifierV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ExtIdleNotifierV1>,
        _data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        !crate::trust::is_sandboxed(&client)
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _notifier: &ExtIdleNotifierV1,
        request: ext_idle_notifier_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let (id, timeout, obeys_inhibitors) = match request {
            ext_idle_notifier_v1::Request::GetIdleNotification { id, timeout, .. } => {
                (id, timeout, true)
            }
            ext_idle_notifier_v1::Request::GetInputIdleNotification { id, timeout, .. } => {
                (id, timeout, false)
            }
            _ => return,
        };
        let notification = data_init.init(id, ());
        state.idle.watches.push(Watch {
            notification,
            timeout: Duration::from_millis(u64::from(timeout)),
            obeys_inhibitors,
            idled: false,
            since: None,
        });
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        notification: &ExtIdleNotificationV1,
        request: ext_idle_notification_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_idle_notification_v1::Request::Destroy = request {
            state
                .idle
                .watches
                .retain(|watch| watch.notification != *notification);
        }
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.push(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        if let Some(at) = self
            .idle
            .inhibitors
            .iter()
            .position(|inhibitor| *inhibitor == surface)
        {
            self.idle.inhibitors.remove(at);
        }
    }
}

smithay::delegate_idle_inhibit!(State);
