use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

use crate::{Globals, Headless, TestClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleEvent {
    Idled,
    Resumed,
}

#[derive(Default)]
pub(crate) struct Idle {
    watches: Vec<(ExtIdleNotificationV1, Vec<IdleEvent>)>,
}

impl TestClient {
    pub fn watch_idle(
        &mut self,
        server: &mut Headless,
        timeout_ms: u32,
        input_only: bool,
    ) -> usize {
        let notifier: ExtIdleNotifierV1 = self.bind("ext_idle_notifier_v1", 2);
        let seat = self.globals.input.seat.as_ref().expect("use_seat first");
        let qh = self.queue.handle();
        let notification = if input_only {
            notifier.get_input_idle_notification(timeout_ms, seat, &qh, ())
        } else {
            notifier.get_idle_notification(timeout_ms, seat, &qh, ())
        };
        self.globals.idle.watches.push((notification, Vec::new()));
        self.exchange(server);
        self.globals.idle.watches.len() - 1
    }

    pub fn idle_events(&self, watch: usize) -> &[IdleEvent] {
        &self.globals.idle.watches[watch].1
    }

    pub fn inhibit_idle(&mut self, server: &mut Headless, window: usize) {
        let manager: ZwpIdleInhibitManagerV1 = self.bind("zwp_idle_inhibit_manager_v1", 1);
        let _: ZwpIdleInhibitorV1 =
            manager.create_inhibitor(&self.windows[window].0, &self.queue.handle(), ());
        self.exchange(server);
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        notification: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let happened = match event {
            ext_idle_notification_v1::Event::Idled => IdleEvent::Idled,
            ext_idle_notification_v1::Event::Resumed => IdleEvent::Resumed,
            _ => return,
        };
        if let Some((_, events)) = globals
            .idle
            .watches
            .iter_mut()
            .find(|(known, _)| known == notification)
        {
            events.push(happened);
        }
    }
}

delegate_noop!(Globals: ExtIdleNotifierV1);
delegate_noop!(Globals: ZwpIdleInhibitManagerV1);
delegate_noop!(Globals: ZwpIdleInhibitorV1);
