use std::time::{Duration, Instant};

use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::reexports::wayland_server::WEnum;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::with_states;
use smithay::wayland::foreign_toplevel_list::{
    ForeignToplevelListHandler, ForeignToplevelListState,
};
use smithay::wayland::shell::xdg::XdgToplevelSurfaceData;
use smithay::wayland::text_input::TextInputSeat;
use unbound_protocols::server::unbound_shell_v1::{self, UnboundShellV1};

use crate::session::SessionRole;
use crate::state::{ClientState, State};

const INSERT_PATIENCE: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub(crate) struct PanelAnchor {
    panel: unbound_shell_v1::Panel,
    output: String,
    edge: unbound_shell_v1::Edge,
    rect: Rectangle<i32, Logical>,
    zone: Point<i32, Logical>,
}

pub(crate) struct PendingInsert {
    text: String,
    requester: ClientId,
    until: Instant,
}

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

    pub(crate) fn scrim_pressed(&self, owner: &WlSurface) {
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_SCRIM_PRESSED_SINCE
                && shell.client() == owner.client()
            {
                shell.scrim_pressed(owner);
            }
        }
    }

    pub(crate) fn open_launcher(&mut self, mode: unbound_shell_v1::LauncherMode) {
        let mut text_field = false;
        self.seat
            .text_input()
            .with_active_text_input(|_, _| text_field = true);
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_OPEN_LAUNCHER_SINCE {
                shell.open_launcher(mode, u32::from(text_field));
            }
        }
    }

    pub(crate) fn insert_text(&mut self, text: String, requester: ClientId) {
        self.pending_insert = Some(PendingInsert {
            text,
            requester,
            until: Instant::now() + INSERT_PATIENCE,
        });
        self.deliver_pending_insert();
    }

    pub(crate) fn deliver_pending_insert(&mut self) {
        let Some(pending) = self.pending_insert.take() else {
            return;
        };
        if Instant::now() > pending.until {
            return;
        }
        let handle = self.seat.text_input();
        let mut inserted = false;
        handle.with_active_text_input(|text_input, surface| {
            if surface.client().map(|client| client.id()) != Some(pending.requester.clone()) {
                text_input.commit_string(Some(pending.text.clone()));
                inserted = true;
            }
        });
        if inserted {
            handle.done(false);
        } else {
            self.pending_insert = Some(pending);
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

    pub(crate) fn media_key(&mut self, key: unbound_shell_v1::MediaKey) {
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_MEDIA_KEY_SINCE {
                shell.media_key(key);
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

    fn set_panel_anchor(
        &mut self,
        surface: &WlSurface,
        panel: unbound_shell_v1::Panel,
        edge: unbound_shell_v1::Edge,
        rect: Rectangle<i32, Logical>,
    ) {
        let Some(screen) = self.layer_screen(surface) else {
            return;
        };
        let output = self.screens[screen].name();
        let zone = self.zone_origin(screen);
        let origin = {
            let map = layer_map_for_output(&self.screens[screen].output);
            let Some(geometry) = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .and_then(|layer| map.layer_geometry(layer))
            else {
                return;
            };
            geometry.loc
        };
        let anchor = PanelAnchor {
            panel,
            output,
            edge,
            rect: Rectangle::new(origin + rect.loc, rect.size),
            zone,
        };
        self.panel_anchors
            .retain(|known| known.panel != panel || known.output != anchor.output);
        for shell in &self.shell_resources {
            send_panel_anchor(shell, &anchor);
        }
        self.panel_anchors.push(anchor);
    }

    fn zone_origin(&self, screen: usize) -> Point<i32, Logical> {
        self.usable_area(screen).location - self.screen_rect(screen).location
    }

    pub(crate) fn follow_panel_zones(&mut self) {
        for index in 0..self.panel_anchors.len() {
            let Some(screen) = self.screen_named(&self.panel_anchors[index].output) else {
                continue;
            };
            let zone = self.zone_origin(screen);
            if self.panel_anchors[index].zone == zone {
                continue;
            }
            self.panel_anchors[index].zone = zone;
            for shell in &self.shell_resources {
                send_panel_anchor(shell, &self.panel_anchors[index]);
            }
        }
    }

    pub(crate) fn shown_displays(&self) -> Vec<String> {
        self.screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .map(|screen| screen.name())
            .collect()
    }

    pub(crate) fn announce_displays(&mut self) {
        let displays = self.shown_displays();
        if displays == self.announced_displays {
            return;
        }
        let names = displays.join(",");
        for shell in &self.shell_resources {
            if shell.version() >= unbound_shell_v1::EVT_DISPLAYS_SINCE {
                shell.displays(names.clone());
            }
        }
        self.announced_displays = displays;
    }

    pub(crate) fn announce_full_screen(&mut self) {
        let states: Vec<(String, bool)> = self
            .screens
            .iter()
            .map(|screen| {
                let workspaces = &screen.workspaces;
                (
                    screen.name(),
                    workspaces.is_full_screen(workspaces.active_id()),
                )
            })
            .collect();
        for state in &states {
            if self.announced_full_screen.contains(state) {
                continue;
            }
            for shell in &self.shell_resources {
                if shell.version() >= unbound_shell_v1::EVT_FULL_SCREEN_SINCE {
                    shell.full_screen(state.0.clone(), u32::from(state.1));
                }
            }
        }
        self.announced_full_screen = states;
    }

    pub(crate) fn announce_active_app(&mut self, key: Option<&WlSurface>) {
        let app_id = key
            .map(|surface| toplevel_names(surface).1)
            .or_else(|| {
                self.windowless_app
                    .as_ref()
                    .map(|(_, app_id)| app_id.clone())
            })
            .unwrap_or_default();
        let output = key
            .and_then(|surface| self.window_index(surface))
            .map(|index| self.screens[self.screen_of(&self.windows[index])].name())
            .unwrap_or_default();
        if output != self.active_output {
            for shell in &self.shell_resources {
                if shell.version() >= unbound_shell_v1::EVT_ACTIVE_OUTPUT_SINCE {
                    shell.active_output(output.clone());
                }
            }
            self.active_output = output;
        }
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
                self.windows[index].announced_workspace = None;
                self.announce_workspaces();
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

impl State {
    pub(crate) fn announce_input_sources_to(&self, shell: &UnboundShellV1) {
        if shell.version() >= unbound_shell_v1::EVT_INPUT_SOURCES_SINCE {
            let sources = &self.input_sources;
            shell.input_sources(sources.codes().join(","), sources.active() as u32);
        }
    }

    pub(crate) fn announce_input_sources(&self) {
        for shell in &self.shell_resources {
            self.announce_input_sources_to(shell);
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
        if shell.version() >= unbound_shell_v1::EVT_ACTIVE_OUTPUT_SINCE {
            shell.active_output(state.active_output.clone());
        }
        state.announce_unresponsive(&shell);
        state.announce_minimized_windows(&shell);
        state.announce_workspaces_to(&shell);
        state.announce_input_sources_to(&shell);
        for anchor in &state.panel_anchors {
            send_panel_anchor(&shell, anchor);
        }
        if shell.version() >= unbound_shell_v1::EVT_FULL_SCREEN_SINCE {
            for (output, full_screen) in &state.announced_full_screen {
                shell.full_screen(output.clone(), u32::from(*full_screen));
            }
        }
        if shell.version() >= unbound_shell_v1::EVT_DISPLAYS_SINCE {
            shell.displays(state.shown_displays().join(","));
        }
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
            unbound_shell_v1::Request::InsertText { text } => {
                if let Some(client) = resource.client() {
                    state.insert_text(text, client.id());
                }
            }
            unbound_shell_v1::Request::ToggleControlCenter => state.toggle_control_center(),
            unbound_shell_v1::Request::ShowLauncher => state.show_launcher(),
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
            unbound_shell_v1::Request::SetScrim { surface, opacity } => {
                crate::effect::set_scrim(&surface, opacity);
                state.needs_redraw = true;
            }
            unbound_shell_v1::Request::SetCornerRadius { surface, radius } => {
                crate::effect::set_corner_radius(&surface, radius);
                state.needs_redraw = true;
            }
            unbound_shell_v1::Request::ShowWindows { app_id } => {
                state.toggle_overview(Some(app_id).filter(|app_id| !app_id.is_empty()));
            }
            unbound_shell_v1::Request::SelectInputSource { index } => {
                state.select_input_source(index as usize);
            }
            unbound_shell_v1::Request::SwitchWorkspace { index } => {
                let screen = state.pointer_screen();
                if (index as usize) < state.screens[screen].workspaces.count() {
                    state.switch_workspace(screen, index as usize);
                }
            }
            unbound_shell_v1::Request::SetPanelAnchor {
                surface,
                panel: WEnum::Value(panel),
                edge: WEnum::Value(edge),
                x,
                y,
                width,
                height,
            } => state.set_panel_anchor(
                &surface,
                panel,
                edge,
                Rectangle::new((x, y).into(), (width, height).into()),
            ),
            unbound_shell_v1::Request::ConfirmSessionEnd {
                ending: WEnum::Value(ending),
            } => state.session_end_requested = Some(ending),
            unbound_shell_v1::Request::QuitAllApps => state.quit_all_apps(resource.clone()),
            unbound_shell_v1::Request::Destroy => {
                state.shell_resources.retain(|shell| shell != resource);
                state.quitting.retain(|(shell, _)| shell != resource);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &UnboundShellV1, _data: &()) {
        state.shell_resources.retain(|shell| shell != resource);
        state.quitting.retain(|(shell, _)| shell != resource);
    }
}

fn send_panel_anchor(shell: &UnboundShellV1, anchor: &PanelAnchor) {
    if shell.version() >= unbound_shell_v1::EVT_PANEL_ANCHOR_SINCE {
        shell.panel_anchor(
            anchor.panel,
            anchor.output.clone(),
            anchor.edge,
            anchor.rect.loc.x - anchor.zone.x,
            anchor.rect.loc.y - anchor.zone.y,
            anchor.rect.size.w,
            anchor.rect.size.h,
        );
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.foreign_toplevel_state
    }
}

smithay::delegate_foreign_toplevel_list!(State);
