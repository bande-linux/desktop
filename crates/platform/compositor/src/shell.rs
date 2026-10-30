use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::Rectangle;
use smithay::wayland::compositor::with_states;
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelListHandler, ForeignToplevelListState,
};
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use unbound_protocols::server::unbound_shell_v1::{self, UnboundShellV1};

use crate::session::SessionRole;
use crate::state::{ClientState, State};

pub(crate) fn is_shell(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|state| state.role == Some(SessionRole::Shell))
}

pub(crate) fn is_shell_client(display: &DisplayHandle, id: &ClientId) -> bool {
    display
        .backend_handle()
        .get_client_data(id.clone())
        .ok()
        .and_then(|data| {
            data.downcast_ref::<ClientState>()
                .map(|state| state.role == Some(SessionRole::Shell))
        })
        .unwrap_or(false)
}

pub(crate) fn toplevel_names(surface: &WlSurface) -> (String, String) {
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok())
            .map(|data| {
                (
                    data.title.clone().unwrap_or_default(),
                    data.app_id.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    })
}

impl State {
    pub(crate) fn show_launcher(&mut self) {
        for shell in &self.shell_resources {
            shell.show_launcher();
        }
    }

    pub(crate) fn capture_screen(&mut self) {
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_CAPTURE_SCREEN_SINCE {
                shell.capture_screen();
            }
        }
    }

    pub(crate) fn capture_area(&mut self) {
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_CAPTURE_AREA_SINCE {
                shell.capture_area();
            }
        }
    }

    pub(crate) fn volume_key(&mut self, key: unbound_shell_v1::VolumeKey, fine: bool) {
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_VOLUME_KEY_SINCE {
                shell.volume_key(key, u32::from(fine));
            }
        }
    }

    pub(crate) fn brightness_key(&mut self, key: unbound_shell_v1::BrightnessKey, fine: bool) {
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_BRIGHTNESS_KEY_SINCE {
                shell.brightness_key(key, u32::from(fine));
            }
        }
    }

    fn toggle_control_center(&mut self) {
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_TOGGLE_CONTROL_CENTER_SINCE {
                shell.toggle_control_center();
            }
        }
    }

    pub(crate) fn announce_active_app(&mut self, key: Option<&WlSurface>) {
        let app_id = key
            .map(|surface| toplevel_names(surface).1)
            .unwrap_or_default();
        if app_id == self.active_app {
            return;
        }
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_ACTIVE_APP_SINCE {
                shell.active_app(app_id.clone());
            }
        }
        self.active_app = app_id;
    }

    pub(crate) fn focus_client(&mut self, client: ClientId) {
        let app_id = self
            .windows
            .iter()
            .find(|window| {
                window
                    .toplevel
                    .wl_surface()
                    .client()
                    .map(|owner| owner.id())
                    == Some(client.clone())
            })
            .map(|window| toplevel_names(window.toplevel.wl_surface()).1);
        if let Some(app_id) = app_id {
            self.activate_app(&app_id);
        }
    }

    pub(crate) fn activate_app(&mut self, app_id: &str) {
        self.show_app(app_id);
        let (mut chosen, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.windows)
            .into_iter()
            .partition(|window| {
                window.size.is_some() && toplevel_names(window.toplevel.wl_surface()).1 == app_id
            });
        self.windows = rest;
        let restored = chosen
            .iter()
            .all(|window| window.is_minimized())
            .then(|| chosen.iter().max_by_key(|window| window.minimized))
            .flatten()
            .map(|latest| latest.toplevel.wl_surface().clone());
        if let Some(index) = restored
            .as_ref()
            .and_then(|surface| chosen.iter().position(|window| window.surface() == surface))
        {
            let window = chosen.remove(index);
            chosen.push(window);
        }
        self.windows.append(&mut chosen);
        if let Some(restored) = &restored {
            self.unminimize(restored);
        }
        let frontmost = self
            .windows
            .iter()
            .rev()
            .find(|window| {
                window.visible() && toplevel_names(window.toplevel.wl_surface()).1 == app_id
            })
            .map(|window| window.toplevel.wl_surface().clone());
        if let Some(frontmost) = frontmost {
            self.needs_redraw = true;
            if self.exclusive_layer.is_none() {
                self.focus(Some(frontmost));
            }
        }
    }

    pub(crate) fn publish_toplevel(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .windows
            .iter()
            .position(|window| window.toplevel.wl_surface() == surface)
        else {
            return;
        };
        let (title, app_id) = toplevel_names(surface);
        match &self.windows[index].foreign {
            Some(handle) => {
                handle.send_title(&title);
                handle.send_app_id(&app_id);
                handle.send_done();
            }
            None if self.windows[index].size.is_some() => {
                let handle = self
                    .foreign_toplevel_state
                    .new_toplevel::<State>(title, app_id);
                self.windows[index].foreign = Some(handle);
            }
            None => {}
        }
    }

    pub(crate) fn withdraw_toplevel(&mut self, surface: &WlSurface) {
        let handle = self
            .windows
            .iter_mut()
            .find(|window| window.toplevel.wl_surface() == surface)
            .and_then(|window| window.foreign.take());
        if let Some(handle) = handle {
            self.foreign_toplevel_state.remove_toplevel(&handle);
        }
    }
}

impl GlobalDispatch<UnboundShellV1, ()> for State {
    fn bind(
        state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<UnboundShellV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let shell = data_init.init(resource, ());
        if shell.version() >= unbound_shell_v1::EVT_ACTIVE_APP_SINCE {
            shell.active_app(state.active_app.clone());
        }
        state.announce_unresponsive(&shell);
        state.announce_minimized_windows(&shell);
        state.shell_resources.push(shell);
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        is_shell(&client)
    }
}

impl Dispatch<UnboundShellV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &UnboundShellV1,
        request: unbound_shell_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            unbound_shell_v1::Request::ActivateApp { app_id } => state.activate_app(&app_id),
            unbound_shell_v1::Request::ToggleControlCenter => state.toggle_control_center(),
            unbound_shell_v1::Request::SetIconRect {
                app_id,
                surface,
                x,
                y,
                width,
                height,
            } => state.set_icon_rect(
                app_id,
                surface,
                Rectangle::new((x, y).into(), (width, height).into()),
            ),
            unbound_shell_v1::Request::HideApp { app_id } => state.hide_app(&app_id),
            unbound_shell_v1::Request::ShowApp { app_id } => {
                state.show_app(&app_id);
                state.needs_redraw = true;
            }
            unbound_shell_v1::Request::QuitApp { app_id } => state.quit_app_named(&app_id),
            unbound_shell_v1::Request::ForceQuitApp { app_id } => state.force_quit_app(&app_id),
            unbound_shell_v1::Request::ActivateWindow { identifier } => {
                state.activate_window(&identifier);
            }
            unbound_shell_v1::Request::SetCornerRadius { surface, radius } => {
                crate::effect::set_corner_radius(&surface, radius);
                state.needs_redraw = true;
            }
            unbound_shell_v1::Request::Destroy => {
                state.shell_resources.retain(|shell| shell != resource);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &UnboundShellV1, _data: &()) {
        state.shell_resources.retain(|shell| shell != resource);
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_state
    }
}

smithay::delegate_foreign_toplevel_list!(State);
