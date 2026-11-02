use std::time::Duration;

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

struct Idle(bool);

impl Dispatch<WlRegistry, GlobalListContents> for Idle {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for Idle {
    fn event(
        idle: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => idle.0 = true,
            ext_idle_notification_v1::Event::Resumed => idle.0 = false,
            _ => {}
        }
    }
}

delegate_noop!(Idle: ignore WlSeat);
delegate_noop!(Idle: ExtIdleNotifierV1);

pub(crate) fn wait_until_idle(after: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Idle>(&connection)?;
    let handle = queue.handle();
    let seat: WlSeat = globals.bind(&handle, 1..=1, ())?;
    let notifier: ExtIdleNotifierV1 = globals.bind(&handle, 1..=1, ())?;
    let timeout = u32::try_from(after.as_millis()).unwrap_or(u32::MAX);
    let notification = notifier.get_idle_notification(timeout, &seat, &handle, ());
    let mut idle = Idle(false);
    while !idle.0 {
        queue.blocking_dispatch(&mut idle)?;
    }
    notification.destroy();
    notifier.destroy();
    connection.flush()?;
    Ok(())
}
