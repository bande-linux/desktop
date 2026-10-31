use rustix::process::{Pid, Signal, kill_process};
use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::wayland::shell::wlr_layer::KeyboardInteractivity;
use unbound_protocols::server::unbound_shell_v1::UnboundShellV1;
use unbound_spring::Spring;

use crate::motion::{BAR_NAMESPACE, DOCK_NAMESPACE, Look};
use crate::shell::{is_shell, toplevel_names};
use crate::state::State;

impl State {
    fn app_windows(&self, app_id: &str) -> Vec<usize> {
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
                    rect.location.to_f64(),
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
        let own = std::process::id() as i32;
        for client in self.app_clients(app_id) {
            if is_shell(&client) {
                continue;
            }
            let Ok(credentials) = client.get_credentials(&self.display_handle) else {
                tracing::warn!("cannot tell which process runs {app_id}");
                continue;
            };
            if credentials.pid == own {
                continue;
            }
            if let Some(pid) = Pid::from_raw(credentials.pid)
                && let Err(error) = kill_process(pid, Signal::KILL)
            {
                tracing::warn!("cannot force {app_id} to quit: {error}");
            }
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

    pub(crate) fn announce_minimized_windows(&self, shell: &UnboundShellV1) {
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
        self.each_shell(|shell| shell.attention_requested(app_id.clone()));
    }

    pub(crate) fn focus_shell(&mut self, namespace: &str) {
        if self.is_locked() || self.exclusive_layer.is_some() {
            return;
        }
        let pointer = self.pointer_screen();
        let order =
            std::iter::once(pointer).chain((0..self.screens.len()).filter(|s| *s != pointer));
        let order: Vec<usize> = order.collect();
        let find = |wanted: &str| {
            order.iter().find_map(|screen| {
                let map = layer_map_for_output(&self.screens[*screen].output);
                map.layers()
                    .find(|layer| {
                        layer.namespace() == wanted
                            && layer.cached_state().keyboard_interactivity
                                != KeyboardInteractivity::None
                    })
                    .map(|layer| layer.wl_surface().clone())
            })
        };
        let surface = find(namespace)
            .or_else(|| find(BAR_NAMESPACE))
            .or_else(|| find(DOCK_NAMESPACE));

        let Some(surface) = surface else {
            return;
        };
        let owner = surface.client();
        self.focus(Some(surface));
        for shell in &self.shell_resources {
            if shell.client() == owner {
                shell.focus_shell(namespace.to_owned());
            }
        }
    }
}
