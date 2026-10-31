use unbound_compositor::SessionRole;
use unbound_protocols::client::unbound_app_control_v1::{self, UnboundAppControlV1};
use unbound_protocols::client::unbound_app_v1::{self, Placement, UnboundAppV1};
use unbound_protocols::client::unbound_shell_v1::{self, UnboundShellV1};
use wayland_client::protocol::wl_shm::Format;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop, event_created_child,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, ZwlrLayerSurfaceV1,
};
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};

use crate::{Globals, Headless, Memory, SessionEnd, ShmImage, TestClient, Workspace};

#[derive(Default)]
pub(crate) struct Shell {
    layer_size: Option<(u32, u32)>,
    token: Option<String>,
    preferred_scale: Option<u32>,
    fractional_scale: Option<WpFractionalScaleV1>,
    pub(crate) layers: Vec<(WlSurface, ZwlrLayerSurfaceV1)>,
    shell: Option<UnboundShellV1>,
    app_control: Option<UnboundAppControlV1>,
    launcher_requests: usize,
    scrim_presses: usize,
    launcher_modes: Vec<(unbound_shell_v1::LauncherMode, bool)>,
    capture_requests: usize,
    area_capture_requests: usize,
    active_apps: Vec<String>,
    volume_keys: Vec<(unbound_shell_v1::VolumeKey, bool)>,
    brightness_keys: Vec<(unbound_shell_v1::BrightnessKey, bool)>,
    media_keys: Vec<unbound_shell_v1::MediaKey>,
    panel_toggles: Vec<unbound_shell_v1::Panel>,
    responding: Vec<(i32, String, bool)>,
    minimized: Vec<(String, bool)>,
    attention: Vec<String>,
    shell_focus: Vec<String>,
    identifiers: Vec<(ExtForeignToplevelHandleV1, String)>,
    quit_requests: usize,
    apps_running: Vec<(String, u32)>,
    toplevels: Vec<(ExtForeignToplevelHandleV1, String, bool)>,
    toplevel_updates: usize,
    workspaces: Option<(u32, u32)>,
    window_workspaces: Vec<(String, u32)>,
    input_sources: Option<(String, u32)>,
    panel_anchors: Vec<(unbound_shell_v1::Panel, (i32, i32, i32, i32))>,
}

pub struct LayerRequest {
    pub layer: Layer,
    pub anchor: Anchor,
    pub size: (u32, u32),
    pub exclusive_zone: i32,
    pub keyboard: KeyboardInteractivity,
    pub color: [u8; 4],
}

impl TestClient {
    pub fn show_layer(&mut self, server: &mut Headless, request: &LayerRequest) -> (u32, u32) {
        self.show_named_layer(server, request, "unbound-test")
    }

    pub fn show_named_layer(
        &mut self,
        server: &mut Headless,
        request: &LayerRequest,
        namespace: &str,
    ) -> (u32, u32) {
        let qh = self.queue.handle();
        let shell: ZwlrLayerShellV1 = self.bind("zwlr_layer_shell_v1", 4);
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        let layer_surface =
            shell.get_layer_surface(&surface, None, request.layer, namespace.to_owned(), &qh, ());
        layer_surface.set_size(request.size.0, request.size.1);
        layer_surface.set_anchor(request.anchor);
        layer_surface.set_exclusive_zone(request.exclusive_zone);
        layer_surface.set_keyboard_interactivity(request.keyboard);
        self.globals.shell.layer_size = None;
        surface.commit();
        self.exchange(server);
        let (width, height) = self
            .globals
            .shell
            .layer_size
            .expect("the layer surface is configured");
        let pixels = request.color.repeat((width * height) as usize);
        let buffer = self.shm_buffer(&ShmImage {
            width: width as i32,
            height: height as i32,
            stride: (width * 4) as i32,
            format: Format::Argb8888,
            bytes: &pixels,
        });
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.commit();
        self.exchange(server);
        self.globals.shell.layers.push((surface, layer_surface));
        (width, height)
    }

    pub fn connect_as_shell(server: &mut Headless) -> Self {
        let (client_end, server_end) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        server
            .insert_session_client(server_end, SessionRole::Shell)
            .expect("insert shell client");
        Self::from_socket(server, client_end)
    }

    pub fn hide_layer(&mut self, server: &mut Headless, index: usize) {
        let surface = &self.globals.shell.layers[index].0;
        surface.attach(None, 0, 0);
        surface.commit();
        self.exchange(server);
    }

    pub fn declare_layer_opaque(
        &mut self,
        server: &mut Headless,
        index: usize,
        (width, height): (u32, u32),
    ) {
        let surface = &self.globals.shell.layers[index].0;
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let region = compositor.create_region(&self.queue.handle(), ());
        region.add(0, 0, width as i32, height as i32);
        surface.set_opaque_region(Some(&region));
        region.destroy();
        surface.commit();
        self.exchange(server);
    }

    pub fn reconfigure_layer(&mut self, server: &mut Headless, index: usize) -> bool {
        self.globals.shell.layer_size = None;
        self.globals.shell.layers[index].0.commit();
        self.exchange(server);
        self.globals.shell.layer_size.is_some()
    }

    pub fn close_layer(&mut self, server: &mut Headless, index: usize) {
        let (surface, layer_surface) = self.globals.shell.layers.remove(index);
        layer_surface.destroy();
        surface.destroy();
        self.exchange(server);
    }

    pub fn request_layer_frame(&mut self, server: &mut Headless, index: usize) {
        let surface = &self.globals.shell.layers[index].0;
        surface.frame(&self.queue.handle(), ());
        surface.commit();
        self.exchange(server);
    }

    pub fn join_shell(&mut self, server: &mut Headless) {
        let shell: UnboundShellV1 = self.bind("unbound_shell_v1", 18);
        self.globals.shell.shell = Some(shell);
        let _: ExtForeignToplevelListV1 = self.bind("ext_foreign_toplevel_list_v1", 1);
        self.exchange(server);
    }

    pub fn set_scrim(&mut self, server: &mut Headless, index: usize, opacity: f64) {
        if let Some(shell) = &self.globals.shell.shell {
            shell.set_scrim(&self.globals.shell.layers[index].0, opacity);
        }
        self.exchange(server);
    }

    pub fn scrim_presses(&self) -> usize {
        self.globals.shell.scrim_presses
    }

    pub fn launcher_requests(&self) -> usize {
        self.globals.shell.launcher_requests
    }

    pub fn launcher_modes(&self) -> &[(unbound_shell_v1::LauncherMode, bool)] {
        &self.globals.shell.launcher_modes
    }

    pub fn insert_text(&mut self, server: &mut Headless, text: &str) {
        if let Some(shell) = &self.globals.shell.shell {
            shell.insert_text(text.to_owned());
        }
        self.exchange(server);
    }

    pub fn screen_capture_requests(&self) -> usize {
        self.globals.shell.capture_requests
    }

    pub fn area_capture_requests(&self) -> usize {
        self.globals.shell.area_capture_requests
    }

    pub fn app_responding(&self) -> &[(i32, String, bool)] {
        &self.globals.shell.responding
    }

    pub fn active_apps(&self) -> &[String] {
        &self.globals.shell.active_apps
    }

    pub fn volume_keys(&self) -> &[(unbound_shell_v1::VolumeKey, bool)] {
        &self.globals.shell.volume_keys
    }

    pub fn brightness_keys(&self) -> &[(unbound_shell_v1::BrightnessKey, bool)] {
        &self.globals.shell.brightness_keys
    }

    pub fn media_keys(&self) -> &[unbound_shell_v1::MediaKey] {
        &self.globals.shell.media_keys
    }

    pub fn panel_toggles(&self) -> &[unbound_shell_v1::Panel] {
        &self.globals.shell.panel_toggles
    }

    pub fn toggle_control_center(&mut self, server: &mut Headless) {
        let shell = self.globals.shell.shell.as_ref().expect("joined the shell");
        shell.toggle_control_center();
        self.exchange(server);
    }

    pub fn set_icon_rect(
        &mut self,
        server: &mut Headless,
        app_id: &str,
        layer: usize,
        rect: (i32, i32, i32, i32),
    ) {
        let shell = self.globals.shell.shell.as_ref().expect("joined the shell");
        let surface = &self.globals.shell.layers[layer].0;
        shell.set_icon_rect(app_id.to_owned(), surface, rect.0, rect.1, rect.2, rect.3);
        self.exchange(server);
    }

    pub fn set_panel_anchor(
        &mut self,
        server: &mut Headless,
        layer: usize,
        panel: unbound_shell_v1::Panel,
        edge: unbound_shell_v1::Edge,
        rect: (i32, i32, i32, i32),
    ) {
        let shell = self.globals.shell.shell.as_ref().expect("joined the shell");
        let surface = &self.globals.shell.layers[layer].0;
        shell.set_panel_anchor(surface, panel, edge, rect.0, rect.1, rect.2, rect.3);
        self.exchange(server);
    }

    pub fn panel_anchor(&self, panel: unbound_shell_v1::Panel) -> Option<(i32, i32, i32, i32)> {
        self.globals
            .shell
            .panel_anchors
            .iter()
            .rev()
            .find(|(known, _)| *known == panel)
            .map(|(_, rect)| *rect)
    }

    pub fn take_app_requests(&mut self, server: &mut Headless) {
        let _: UnboundAppV1 = self.bind("unbound_app_v1", 1);
        self.exchange(server);
    }

    pub fn place_window(&mut self, server: &mut Headless, index: usize, placement: Placement) {
        let app: UnboundAppV1 = self.bind("unbound_app_v1", 6);
        app.place_window(&self.windows[index].2, placement);
        self.exchange(server);
    }

    pub fn move_to_workspace(&mut self, server: &mut Headless, index: usize, workspace: Workspace) {
        let app: UnboundAppV1 = self.bind("unbound_app_v1", 3);
        app.move_to_workspace(&self.windows[index].2, workspace);
        self.exchange(server);
    }

    pub fn keep_on_top(&mut self, server: &mut Headless, index: usize, keep: bool) {
        let app: UnboundAppV1 = self.bind("unbound_app_v1", 4);
        app.set_keep_on_top(&self.windows[index].2, u32::from(keep));
        self.exchange(server);
    }

    pub fn workspaces(&self) -> Option<(u32, u32)> {
        self.globals.shell.workspaces
    }

    pub fn window_workspace(&self, identifier: &str) -> Option<u32> {
        self.globals
            .shell
            .window_workspaces
            .iter()
            .rev()
            .find(|(known, _)| known == identifier)
            .map(|(_, workspace)| *workspace)
    }

    pub fn input_sources(&self) -> Option<(&str, u32)> {
        let (sources, active) = self.globals.shell.input_sources.as_ref()?;
        Some((sources, *active))
    }

    pub fn select_input_source(&mut self, server: &mut Headless, index: u32) {
        self.shell_request(server, |shell| shell.select_input_source(index));
    }

    pub fn show_windows(&mut self, server: &mut Headless, app_id: &str) {
        self.shell_request(server, |shell| shell.show_windows(app_id.to_owned()));
    }

    pub fn quit_requests(&self) -> usize {
        self.globals.shell.quit_requests
    }

    fn shell_request(&mut self, server: &mut Headless, request: impl FnOnce(&UnboundShellV1)) {
        request(self.globals.shell.shell.as_ref().expect("joined the shell"));
        self.exchange(server);
    }

    pub fn join_app_control(&mut self, server: &mut Headless) {
        self.globals.shell.app_control = Some(self.bind("unbound_app_control_v1", 1));
        self.exchange(server);
    }

    pub fn ask_app_to_quit(&mut self, server: &mut Headless, app_id: &str) {
        if let Some(control) = &self.globals.shell.app_control {
            control.quit_app(app_id.to_owned());
        }
        self.exchange(server);
    }

    pub fn hide_app(&mut self, server: &mut Headless, app_id: &str) {
        self.shell_request(server, |shell| shell.hide_app(app_id.to_owned()));
    }

    pub fn show_app(&mut self, server: &mut Headless, app_id: &str) {
        self.shell_request(server, |shell| shell.show_app(app_id.to_owned()));
    }

    pub fn quit_app(&mut self, server: &mut Headless, app_id: &str) {
        self.shell_request(server, |shell| shell.quit_app(app_id.to_owned()));
    }

    pub fn quit_all_apps(&mut self, server: &mut Headless) {
        self.shell_request(server, UnboundShellV1::quit_all_apps);
    }

    pub fn confirm_session_end(&mut self, server: &mut Headless, ending: SessionEnd) {
        self.shell_request(server, |shell| shell.confirm_session_end(ending));
    }

    pub fn apps_running(&self) -> &[(String, u32)] {
        &self.globals.shell.apps_running
    }

    pub fn activate_window(&mut self, server: &mut Headless, identifier: &str) {
        self.shell_request(server, |shell| shell.activate_window(identifier.to_owned()));
    }

    pub fn set_corner_radius(&mut self, server: &mut Headless, layer: usize, radius: u32) {
        let surface = self.globals.shell.layers[layer].0.clone();
        self.shell_request(server, |shell| shell.set_corner_radius(&surface, radius));
    }

    pub(crate) fn toplevel_handle(&self, app_id: &str) -> Option<ExtForeignToplevelHandleV1> {
        self.globals
            .shell
            .toplevels
            .iter()
            .find(|(_, known, closed)| known == app_id && !closed)
            .map(|(handle, _, _)| handle.clone())
    }

    pub fn window_identifiers(&self, app_id: &str) -> Vec<String> {
        let shell = &self.globals.shell;
        shell
            .toplevels
            .iter()
            .filter(|(_, known, closed)| known == app_id && !closed)
            .filter_map(|(handle, _, _)| {
                shell
                    .identifiers
                    .iter()
                    .find(|(known, _)| known == handle)
                    .map(|(_, identifier)| identifier.clone())
            })
            .collect()
    }

    pub fn layer_has_keyboard(&self, index: usize) -> bool {
        self.globals.input.keyboard_surface() == Some(&self.globals.shell.layers[index].0)
    }

    pub fn minimized_windows(&self) -> &[(String, bool)] {
        &self.globals.shell.minimized
    }

    pub fn attention_requests(&self) -> &[String] {
        &self.globals.shell.attention
    }

    pub fn shell_focus_requests(&self) -> &[String] {
        &self.globals.shell.shell_focus
    }

    pub fn activate_app(&mut self, server: &mut Headless, app_id: &str) {
        self.shell_request(server, |shell| shell.activate_app(app_id.to_owned()));
    }

    pub fn toplevel_updates(&self) -> usize {
        self.globals.shell.toplevel_updates
    }

    pub fn running_app_ids(&self) -> Vec<String> {
        self.globals
            .shell
            .toplevels
            .iter()
            .filter(|(_, _, closed)| !closed)
            .map(|(_, app_id, _)| app_id.clone())
            .collect()
    }

    pub fn show_window_of(&mut self, server: &mut Headless, image: &ShmImage, app_id: &str) {
        self.map_window(server, image, Memory::Shm, |_, _, toplevel| {
            toplevel.set_app_id(app_id.to_owned());
        });
    }

    pub fn set_app_id(&mut self, server: &mut Headless, index: usize, app_id: &str) {
        self.windows[index].2.set_app_id(app_id.to_owned());
        self.exchange(server);
    }

    pub fn activation_token(&mut self, server: &mut Headless) -> String {
        let activation: XdgActivationV1 = self.bind("xdg_activation_v1", 1);
        let token = activation.get_activation_token(&self.queue.handle(), ());
        if let Some((surface, _, _)) = self.windows.last() {
            token.set_surface(surface);
        }
        self.globals.shell.token = None;
        token.commit();
        self.exchange(server);
        self.globals
            .shell
            .token
            .take()
            .expect("an activation token")
    }

    pub fn activate(&mut self, server: &mut Headless, index: usize, token: &str) {
        let activation: XdgActivationV1 = self.bind("xdg_activation_v1", 1);
        activation.activate(token.to_owned(), &self.windows[index].0);
        self.exchange(server);
    }

    pub fn show_window_activated(&mut self, server: &mut Headless, image: &ShmImage, token: &str) {
        self.map_window(server, image, Memory::Shm, |client, surface, _| {
            let activation: XdgActivationV1 = client.bind("xdg_activation_v1", 1);
            activation.activate(token.to_owned(), surface);
        });
    }

    pub fn show_window_scaled(
        &mut self,
        server: &mut Headless,
        image: &ShmImage,
        destination: (i32, i32),
    ) {
        self.map_window(server, image, Memory::Shm, |client, surface, _| {
            let viewporter: WpViewporter = client.bind("wp_viewporter", 1);
            let viewport = viewporter.get_viewport(surface, &client.queue.handle(), ());
            viewport.set_destination(destination.0, destination.1);
        });
    }

    pub fn preferred_scale(&mut self, server: &mut Headless, index: usize) -> Option<u32> {
        if self.globals.shell.fractional_scale.is_none() {
            let manager: WpFractionalScaleManagerV1 =
                self.bind("wp_fractional_scale_manager_v1", 1);
            self.globals.shell.fractional_scale = Some(manager.get_fractional_scale(
                &self.windows[index].0,
                &self.queue.handle(),
                (),
            ));
        }
        self.exchange(server);
        self.globals.shell.preferred_scale
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer_surface.ack_configure(serial);
            globals.shell.layer_size = Some((width, height));
        }
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            globals.shell.token = Some(token);
        }
    }
}

impl Dispatch<WpFractionalScaleV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            globals.shell.preferred_scale = Some(scale);
        }
    }
}

delegate_noop!(Globals: ignore ZwlrLayerShellV1);
delegate_noop!(Globals: ignore XdgActivationV1);
delegate_noop!(Globals: ignore WpViewporter);
delegate_noop!(Globals: ignore WpViewport);
delegate_noop!(Globals: ignore WpFractionalScaleManagerV1);

impl Dispatch<UnboundShellV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &UnboundShellV1,
        event: unbound_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            unbound_shell_v1::Event::ShowLauncher => globals.shell.launcher_requests += 1,
            unbound_shell_v1::Event::ScrimPressed { .. } => globals.shell.scrim_presses += 1,
            unbound_shell_v1::Event::OpenLauncher {
                mode: WEnum::Value(mode),
                text_field,
            } => globals.shell.launcher_modes.push((mode, text_field != 0)),
            unbound_shell_v1::Event::CaptureScreen => globals.shell.capture_requests += 1,
            unbound_shell_v1::Event::CaptureArea => globals.shell.area_capture_requests += 1,
            unbound_shell_v1::Event::ActiveApp { app_id } => globals.shell.active_apps.push(app_id),
            unbound_shell_v1::Event::VolumeKey {
                key: WEnum::Value(key),
                fine,
            } => globals.shell.volume_keys.push((key, fine != 0)),
            unbound_shell_v1::Event::BrightnessKey {
                key: WEnum::Value(key),
                fine,
            } => globals.shell.brightness_keys.push((key, fine != 0)),
            unbound_shell_v1::Event::MediaKey {
                key: WEnum::Value(key),
            } => globals.shell.media_keys.push(key),
            unbound_shell_v1::Event::TogglePanel {
                panel: WEnum::Value(panel),
            } => globals.shell.panel_toggles.push(panel),
            unbound_shell_v1::Event::WindowMinimized {
                identifier,
                minimized,
            } => globals.shell.minimized.push((identifier, minimized != 0)),
            unbound_shell_v1::Event::AttentionRequested { app_id } => {
                globals.shell.attention.push(app_id);
            }
            unbound_shell_v1::Event::InputSources { sources, active } => {
                globals.shell.input_sources = Some((sources, active));
            }
            unbound_shell_v1::Event::Workspaces { count, active } => {
                globals.shell.workspaces = Some((count, active));
            }
            unbound_shell_v1::Event::WindowWorkspace {
                identifier,
                workspace,
            } => globals
                .shell
                .window_workspaces
                .push((identifier, workspace)),
            unbound_shell_v1::Event::PanelAnchor {
                panel: WEnum::Value(panel),
                x,
                y,
                width,
                height,
                ..
            } => globals
                .shell
                .panel_anchors
                .push((panel, (x, y, width, height))),
            unbound_shell_v1::Event::FocusShell { namespace } => {
                globals.shell.shell_focus.push(namespace);
            }
            unbound_shell_v1::Event::AppsRunning { app_id, count } => {
                globals.shell.apps_running.push((app_id, count));
            }
            unbound_shell_v1::Event::AppResponding {
                pid,
                app_id,
                responding,
            } => globals
                .shell
                .responding
                .push((pid, app_id, responding != 0)),
            _ => {}
        }
    }
}

impl Dispatch<UnboundAppV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &UnboundAppV1,
        event: unbound_app_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let unbound_app_v1::Event::QuitRequested = event {
            globals.shell.quit_requests += 1;
        }
    }
}

impl Dispatch<UnboundAppControlV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &UnboundAppControlV1,
        event: unbound_app_control_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let unbound_app_control_v1::Event::AppResponding {
            pid,
            app_id,
            responding,
        } = event
        {
            globals
                .shell
                .responding
                .push((pid, app_id, responding != 0));
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            globals
                .shell
                .toplevels
                .push((toplevel, String::new(), false));
        }
    }

    event_created_child!(Globals, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_handle_v1::Event::Done = &event {
            globals.shell.toplevel_updates += 1;
        }
        if let ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } = &event {
            globals
                .shell
                .identifiers
                .push((handle.clone(), identifier.clone()));
        }
        let Some(entry) = globals
            .shell
            .toplevels
            .iter_mut()
            .find(|(known, _, _)| known == handle)
        else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => entry.1 = app_id,
            ext_foreign_toplevel_handle_v1::Event::Closed => entry.2 = true,
            _ => {}
        }
    }
}
