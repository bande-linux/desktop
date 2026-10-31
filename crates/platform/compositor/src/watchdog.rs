use std::collections::HashSet;
use std::time::Duration;

use smithay::input::pointer::CursorIcon;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::xdg::ShellClient;
use unbound_protocols::server::unbound_app_control_v1::UnboundAppControlV1;
use unbound_protocols::server::unbound_shell_v1::UnboundShellV1;

use crate::shell::toplevel_names;
use crate::state::{Compositor, State};

pub const UNRESPONSIVE_AFTER: Duration = Duration::from_secs(3);
const PING_INTERVAL: Duration = Duration::from_secs(1);

struct Ping {
    owner: ClientId,
    client: ShellClient,
    pid: i32,
    app_id: String,
    sent: Duration,
    pending: bool,
    unresponsive: bool,
}

#[derive(Default)]
pub(crate) struct Watchdog {
    pings: Vec<Ping>,
    interacted: bool,
    everyone: bool,
}

impl Watchdog {
    pub(crate) fn note_interaction(&mut self) {
        self.interacted = true;
    }

    pub(crate) fn ping_everyone(&mut self) {
        self.everyone = true;
    }

    pub(crate) fn is_unresponsive(&self, owner: &ClientId) -> bool {
        self.pings
            .iter()
            .any(|ping| ping.unresponsive && &ping.owner == owner)
    }

    pub(crate) fn has_unresponsive(&self) -> bool {
        self.pings.iter().any(|ping| ping.unresponsive)
    }
}

fn announce(
    shells: &[UnboundShellV1],
    controls: &[UnboundAppControlV1],
    ping: &Ping,
    responding: bool,
) {
    for shell in shells {
        shell.app_responding(ping.pid, ping.app_id.clone(), u32::from(responding));
    }
    for control in controls {
        control.app_responding(ping.pid, ping.app_id.clone(), u32::from(responding));
    }
}

impl State {
    pub(crate) fn announce_unresponsive(
        &self,
        shell: Option<&UnboundShellV1>,
        control: Option<&UnboundAppControlV1>,
    ) {
        for ping in self.watchdog.pings.iter().filter(|ping| ping.unresponsive) {
            announce(
                shell.map(std::slice::from_ref).unwrap_or_default(),
                control.map(std::slice::from_ref).unwrap_or_default(),
                ping,
                false,
            );
        }
    }

    pub(crate) fn pong(&mut self, client: &ShellClient) {
        let Some(ping) = self
            .watchdog
            .pings
            .iter_mut()
            .find(|ping| &ping.client == client && ping.pending)
        else {
            return;
        };
        ping.pending = false;
        if std::mem::take(&mut ping.unresponsive) {
            announce(
                &self.shell_resources,
                &self.app_control_resources,
                ping,
                true,
            );
            self.needs_redraw = true;
        }
    }

    fn interacting_clients(&self) -> HashSet<ClientId> {
        let keyboard = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        let pointer = self
            .seat
            .get_pointer()
            .and_then(|pointer| pointer.current_focus());
        keyboard
            .into_iter()
            .chain(pointer)
            .filter_map(|surface| surface.client())
            .map(|client| client.id())
            .collect()
    }

    fn ping_clients(&mut self, now: Duration) {
        let everyone = std::mem::take(&mut self.watchdog.everyone);
        let interacted = std::mem::take(&mut self.watchdog.interacted);
        if !everyone && !interacted {
            return;
        }
        let interacting = self.interacting_clients();
        let mut pinged: HashSet<ClientId> = self
            .watchdog
            .pings
            .iter()
            .map(|ping| ping.owner.clone())
            .collect();
        for window in &self.windows {
            let Some(owner) = window.toplevel.wl_surface().client() else {
                continue;
            };
            let id = owner.id();
            if (!everyone && !interacting.contains(&id)) || pinged.contains(&id) {
                continue;
            }
            let client = window.toplevel.client();
            if client.send_ping(SERIAL_COUNTER.next_serial()).is_err() {
                continue;
            }
            let pid = owner
                .get_credentials(&self.display_handle)
                .map_or(0, |credentials| credentials.pid);
            pinged.insert(id.clone());
            self.watchdog.pings.push(Ping {
                owner: id,
                client,
                pid,
                app_id: toplevel_names(window.toplevel.wl_surface()).1,
                sent: now,
                pending: true,
                unresponsive: false,
            });
        }
    }

    fn watchdog_due(&mut self, now: Duration) -> Option<Duration> {
        let shells = &self.shell_resources;
        let controls = &self.app_control_resources;
        let mut changed = false;
        self.watchdog.pings.retain(|ping| {
            let alive = ping.client.alive();
            let quiet = !ping.pending && now >= ping.sent + PING_INTERVAL;
            if !alive && ping.unresponsive {
                announce(shells, controls, ping, true);
                changed = true;
            }
            alive && !quiet
        });
        self.ping_clients(now);
        let shells = &self.shell_resources;
        let controls = &self.app_control_resources;
        for ping in &mut self.watchdog.pings {
            if ping.pending && !ping.unresponsive && now >= ping.sent + UNRESPONSIVE_AFTER {
                ping.unresponsive = true;
                announce(shells, controls, ping, false);
                changed = true;
            }
        }
        if changed {
            self.needs_redraw = true;
        }
        if self.watchdog.has_unresponsive() {
            self.load_named_cursor(CursorIcon::Wait);
        }
        self.watchdog
            .pings
            .iter()
            .filter(|ping| ping.pending && !ping.unresponsive)
            .map(|ping| ping.sent + UNRESPONSIVE_AFTER)
            .min()
    }
}

impl Compositor {
    pub fn watchdog_tick(&mut self, now: Duration) -> Option<Duration> {
        self.state.watchdog_due(now)
    }
}
