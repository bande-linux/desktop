use bande_protocols::server::bande_shell_v1::BandeShellV1;
use bande_spring::Spring;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};

use crate::motion::Look;
use crate::shell::toplevel_names;
use crate::state::State;

impl State {
    pub(crate) fn app_windows(&self, app_id: &str) -> Vec<usize> {
        self.windows
            .iter()
            .enumerate()
            .filter(|(_, window)| {
                window.size.is_some() && toplevel_names(window.surface()).1 == app_id
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub(crate) fn app_clients(&self, app_id: &str) -> Vec<Client> {
        let mut clients: Vec<Client> = Vec::new();
        for index in self.app_windows(app_id) {
            if let Some(client) = self.windows[index].surface().client()
                && !clients.contains(&client)
            {
                clients.push(client);
            }
        }
        for (client, windowless) in self.windowless_apps() {
            if windowless == app_id && !clients.contains(&client) {
                clients.push(client);
            }
        }
        clients
    }

    fn app_surfaces(&self, app_id: &str) -> Vec<WlSurface> {
        self.app_windows(app_id)
            .into_iter()
            .map(|index| self.windows[index].surface().clone())
            .collect()
    }

    pub(crate) fn hide_app(&mut self, app_id: &str) {
        self.hide_windows(self.app_surfaces(app_id));
        if self
            .windowless_app
            .as_ref()
            .is_some_and(|(_, windowless)| windowless == app_id)
        {
            self.windowless_app = None;
            let next = self.frontmost(None);
            self.focus(next);
        }
    }

    pub(crate) fn hide_windows(&mut self, surfaces: Vec<WlSurface>) {
        for surface in surfaces {
            let Some(index) = self.window_index(&surface) else {
                continue;
            };
            if std::mem::replace(&mut self.windows[index].hidden, true) {
                continue;
            }
            self.animate_window(index, Spring::FADE, None);
            self.refocus_after_losing(&surface);
        }
        self.keep_families_together();
        self.needs_redraw = true;
    }

    pub(crate) fn show_app(&mut self, app_id: &str) {
        self.show_windows(self.app_surfaces(app_id));
    }

    pub(crate) fn show_windows(&mut self, surfaces: Vec<WlSurface>) {
        for surface in surfaces {
            let Some(index) = self.window_index(&surface) else {
                continue;
            };
            if !std::mem::take(&mut self.windows[index].hidden) {
                continue;
            }
            let from = self.windows[index].geometry().map(|rect| {
                Look::hidden(smithay::utils::Rectangle::new(
                    rect.loc.to_f64(),
                    rect.size.to_f64(),
                ))
            });
            self.animate_window(index, Spring::FADE, from);
        }
        self.keep_families_together();
    }

    pub(crate) fn quit_app_named(&mut self, app_id: &str) {
        for client in self.app_clients(app_id) {
            self.quit_app(Some(client));
        }
    }

    pub(crate) fn force_quit_app(&mut self, app_id: &str) {
        let ids = self.app_ids(app_id);
        for client in self.app_clients(app_id) {
            self.kill_client(&client, &ids);
        }
    }

    pub(crate) fn activate_window(&mut self, identifier: &str) {
        let Some(index) = self.windows.iter().position(|window| {
            window
                .foreign
                .as_ref()
                .is_some_and(|handle| handle.identifier() == identifier)
        }) else {
            return;
        };
        let surface = self.windows[index].surface().clone();
        self.bring_forward(&surface);
    }

    pub(crate) fn announce_minimized(&self, index: usize, minimized: bool) {
        let Some(handle) = &self.windows[index].foreign else {
            return;
        };
        self.each_shell(|shell| shell.window_minimized(handle.identifier(), u32::from(minimized)));
    }

    pub(crate) fn announce_minimized_windows(&self, shell: &BandeShellV1) {
        for window in self
            .windows
            .iter()
            .filter(|window| window.is_minimized() && window.toplevel.parent().is_none())
        {
            if let Some(handle) = &window.foreign {
                shell.window_minimized(handle.identifier(), 1);
            }
        }
    }

    pub(crate) fn request_attention(&self, app_id: String) {
        if app_id == self.active_app {
            return;
        }
        self.each_shell(|shell| shell.attention_requested(app_id.clone()));
    }
}
