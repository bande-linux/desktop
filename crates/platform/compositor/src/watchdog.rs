use std::collections::HashSet;
use std::time::Duration;

use bande_protocols::server::bande_app_control_v1::BandeAppControlV1;
use bande_protocols::server::bande_shell_v1::BandeShellV1;
use smithay::input::pointer::CursorIcon;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::shell::xdg::ShellClient;

use crate::shell::toplevel_names;
use crate::state::{Compositor, State, client_pid};

pub const UNRESPONSIVE_AFTER: Duration = Duration::from_secs(2);
const MICROHANG_AFTER: Duration = Duration::from_millis(250);
pub(crate) const PING_SINCE: u32 = 2;
const PING_INTERVAL: Duration = Duration::from_secs(1);
const QUIT_PATIENCE: Duration = Duration::from_secs(10);
const REPORT_HANG_AFTER: Duration = Duration::from_secs(10);

#[derive(PartialEq)]
enum Pinged {
    App(ShellClient),
    Shell(BandeShellV1),
}

impl Ping {
    fn hang_token(&self) -> u64 {
        u64::try_from(self.sent.as_millis()).unwrap_or(u64::MAX)
    }

    fn reported_pid(&self) -> Option<u32> {
        u32::try_from(self.pid).ok().filter(|pid| *pid > 0)
    }

    fn microhang_counter(&self) -> String {
        match self.pinged {
            Pinged::App(_) if !self.app_id.is_empty() => format!("microhang:{}", self.app_id),
            Pinged::App(_) => "microhang:app".to_owned(),
            Pinged::Shell(_) => "microhang:shell".to_owned(),
        }
    }

    fn measure_answer(&mut self, now: Duration) {
        if !std::mem::take(&mut self.answered) {
            return;
        }
        let waited = now.saturating_sub(self.sent);
        if waited >= MICROHANG_AFTER {
            bande_log::fault::count(&self.microhang_counter());
        }
        if self.reported
            && let Some(pid) = self.reported_pid()
        {
            bande_log::fault::hang_over(pid, self.hang_token(), waited.as_secs_f64(), true);
        }
    }
}

impl Pinged {
    fn alive(&self) -> bool {
        match self {
            Self::App(client) => client.alive(),
            Self::Shell(shell) => shell.is_alive(),
        }
    }
}

struct Ping {
    owner: ClientId,
    pinged: Pinged,
    pid: i32,
    app_id: String,
    sent: Duration,
    pending: bool,
    answered: bool,
    unresponsive: bool,
    given_up: bool,
    reported: bool,
}

#[derive(Default)]
pub(crate) struct Watchdog {
    pings: Vec<Ping>,
    interacted: bool,
    everyone: bool,
    quit_asked: Option<Duration>,
    quit_patience_over: bool,
}

impl Watchdog {
    pub(crate) fn note_interaction(&mut self) {
        self.interacted = true;
    }

    pub(crate) fn ping_everyone(&mut self) {
        self.everyone = true;
    }

    pub(crate) fn unresponsive(&self) -> usize {
        self.pings.iter().filter(|ping| ping.unresponsive).count()
    }

    pub(crate) fn is_unresponsive(&self, owner: &ClientId) -> bool {
        self.pings
            .iter()
            .any(|ping| ping.unresponsive && &ping.owner == owner)
    }

    pub(crate) fn gave_up_on(&self, owner: &ClientId, windowless: bool) -> bool {
        (windowless && self.quit_patience_over)
            || self
                .pings
                .iter()
                .any(|ping| ping.given_up && &ping.owner == owner)
    }

    pub(crate) fn has_unresponsive(&self) -> bool {
        self.pings.iter().any(|ping| ping.unresponsive)
    }

    pub(crate) fn hung_since(&self, pid: i32) -> Option<Duration> {
        self.pings
            .iter()
            .find(|ping| ping.unresponsive && ping.pid == pid)
            .map(|ping| ping.sent)
    }
}

fn announce(
    shells: &[BandeShellV1],
    controls: &[BandeAppControlV1],
    ping: &Ping,
    responding: bool,
) {
    if matches!(ping.pinged, Pinged::Shell(_)) {
        return;
    }
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
        shell: Option<&BandeShellV1>,
        control: Option<&BandeAppControlV1>,
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

    pub(crate) fn pong(&mut self, client: ShellClient) {
        self.answered(&Pinged::App(client));
    }

    pub(crate) fn shell_pong(&mut self, shell: BandeShellV1) {
        self.answered(&Pinged::Shell(shell));
    }

    fn answered(&mut self, pinged: &Pinged) {
        let Some(ping) = self
            .watchdog
            .pings
            .iter_mut()
            .find(|ping| &ping.pinged == pinged && ping.pending)
        else {
            return;
        };
        ping.pending = false;
        ping.answered = true;
        ping.given_up = false;
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
            let pid = client_pid(&owner).unwrap_or(0);
            pinged.insert(id.clone());
            self.watchdog.pings.push(Ping {
                owner: id,
                pinged: Pinged::App(client),
                pid,
                app_id: toplevel_names(window.toplevel.wl_surface()).1,
                sent: now,
                pending: true,
                answered: false,
                unresponsive: false,
                given_up: false,
                reported: false,
            });
        }
        for shell in self
            .shell_resources
            .iter()
            .filter(|shell| shell.version() >= PING_SINCE)
        {
            let Some(owner) = shell.client() else {
                continue;
            };
            let id = owner.id();
            if (!everyone && !interacting.contains(&id)) || pinged.contains(&id) {
                continue;
            }
            shell.ping(SERIAL_COUNTER.next_serial().into());
            let pid = client_pid(&owner).unwrap_or(0);
            pinged.insert(id.clone());
            self.watchdog.pings.push(Ping {
                owner: id,
                pinged: Pinged::Shell(shell.clone()),
                pid,
                app_id: String::new(),
                sent: now,
                pending: true,
                answered: false,
                unresponsive: false,
                given_up: false,
                reported: false,
            });
        }
    }

    fn watchdog_due(&mut self, now: Duration) -> Option<Duration> {
        let shells = &self.shell_resources;
        let controls = &self.app_control_resources;
        let mut changed = false;
        for ping in &mut self.watchdog.pings {
            ping.measure_answer(now);
        }
        self.watchdog.pings.retain(|ping| {
            let alive = ping.pinged.alive();
            let quiet = !ping.pending && now >= ping.sent + PING_INTERVAL;
            if !alive && ping.unresponsive {
                announce(shells, controls, ping, true);
                changed = true;
            }
            if !alive
                && ping.pending
                && ping.reported
                && let Some(pid) = ping.reported_pid()
            {
                bande_log::fault::hang_over(
                    pid,
                    ping.hang_token(),
                    now.saturating_sub(ping.sent).as_secs_f64(),
                    false,
                );
            }
            alive && !quiet
        });
        let waiting_for_apps = !self.quitting.is_empty() && !self.app_owners().is_empty();
        self.watchdog.everyone |= waiting_for_apps;
        self.ping_clients(now);
        let shells = &self.shell_resources;
        let controls = &self.app_control_resources;
        for ping in &mut self.watchdog.pings {
            if ping.pending && !ping.unresponsive && now >= ping.sent + UNRESPONSIVE_AFTER {
                ping.unresponsive = true;
                announce(shells, controls, ping, false);
                changed = true;
            }
            ping.given_up =
                waiting_for_apps && ping.unresponsive && now >= ping.sent + QUIT_PATIENCE;
            if ping.unresponsive && !ping.reported && now >= ping.sent + REPORT_HANG_AFTER {
                ping.reported = true;
                if let Some(pid) = ping.reported_pid() {
                    let kind = match ping.pinged {
                        Pinged::App(_) => "app-hang",
                        Pinged::Shell(_) => "shell-stall",
                    };
                    bande_log::fault::report_hang_of(
                        kind,
                        pid,
                        now.saturating_sub(ping.sent).as_secs(),
                        ping.hang_token(),
                    );
                }
            }
        }
        self.watchdog.quit_asked =
            waiting_for_apps.then(|| self.watchdog.quit_asked.unwrap_or(now));
        self.watchdog.quit_patience_over = self
            .watchdog
            .quit_asked
            .is_some_and(|asked| now >= asked + QUIT_PATIENCE);
        self.announce_apps_running();
        if changed {
            self.needs_redraw = true;
        }
        if self.watchdog.has_unresponsive() {
            self.load_named_cursor(CursorIcon::Wait);
        }
        self.watchdog
            .pings
            .iter()
            .filter(|ping| ping.pending && !ping.reported)
            .map(|ping| {
                ping.sent
                    + if ping.unresponsive {
                        REPORT_HANG_AFTER
                    } else {
                        UNRESPONSIVE_AFTER
                    }
            })
            .min()
            .into_iter()
            .chain(waiting_for_apps.then_some(now + PING_INTERVAL))
            .min()
    }
}

impl Compositor {
    pub fn watchdog_tick(&mut self, now: Duration) -> Option<Duration> {
        let due = self.state.watchdog_due(now);
        due.into_iter()
            .chain(self.relaunch_hung_lock_screen(now))
            .min()
    }
}
