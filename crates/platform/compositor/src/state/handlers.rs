use std::sync::OnceLock;
use std::time::Duration;

use super::{ClientState, MENU_BAR_NAMESPACE, State, is_mapped, send_preferred_scale};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Modifier};
use smithay::backend::renderer::utils::on_commit_buffer_handler;
use smithay::desktop::{LayerSurface as MappedLayer, WindowSurfaceType, layer_map_for_output};
use smithay::input::pointer::CursorImageStatus;

use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::Serial;
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, with_states,
};
use smithay::wayland::dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier};
use smithay::wayland::fractional_scale::FractionalScaleHandler;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::set_data_device_focus;
use smithay::wayland::selection::ext_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
};
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceData, WlrLayerShellHandler,
    WlrLayerShellState,
};
use smithay::wayland::shell::xdg::{
    Configure, PopupSurface, PositionerState, ShellClient, ToplevelSurface, XdgShellHandler,
    XdgShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};
use smithay::{
    delegate_compositor, delegate_cursor_shape, delegate_data_device, delegate_dmabuf,
    delegate_ext_data_control, delegate_fractional_scale, delegate_layer_shell, delegate_output,
    delegate_presentation, delegate_primary_selection, delegate_seat, delegate_shm,
    delegate_viewporter, delegate_xdg_activation, delegate_xdg_shell,
};

use crate::shell;
use crate::window::Window;
use smithay::wayland::tablet_manager::TabletSeatHandler;

const ACTIVATION_TOKEN_LIFETIME: Duration = Duration::from_secs(60);

impl State {
    fn commit_layer(&mut self, surface: &WlSurface) -> bool {
        let Some(screen) = self.layer_screen(surface) else {
            return false;
        };
        let mut map = layer_map_for_output(&self.screens[screen].output);
        let Some(layer) = map
            .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
            .cloned()
        else {
            return false;
        };
        map.arrange();
        let mapped = is_mapped(surface);
        let exclusive = (mapped
            && matches!(layer.layer(), Layer::Top | Layer::Overlay)
            && layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive)
            .then(|| layer.layer());
        drop(map);
        self.follow_panel_zones();
        self.follow_exclusive_layer(surface, exclusive);
        let configured = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .is_some_and(|data| data.lock().is_ok_and(|data| data.initial_configure_sent))
        });
        if !configured || !mapped {
            layer.layer_surface().send_configure();
        }
        true
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.globals.compositor_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        crate::sync::wait_for_fences(surface);
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        static UNKNOWN_CLIENT: OnceLock<CompositorClientState> = OnceLock::new();
        match client.get_data::<ClientState>() {
            Some(state) => &state.compositor_state,
            None => UNKNOWN_CLIENT.get_or_init(CompositorClientState::default),
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        crate::effect::commit_blur(surface);
        if self.commit_drag_icon(surface) {
            return;
        }
        self.needs_redraw = true;
        if self.commit_lock_surface(surface)
            || self.commit_popup(surface)
            || self.commit_layer(surface)
        {
            return;
        }
        self.commit_window(surface);
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.globals.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        if dmabuf.num_planes() == 1 && dmabuf.format().modifier == Modifier::Linear {
            dmabuf.set_node(self.scanout_node);
            let _ = notifier.successful::<State>();
        } else {
            notifier.failed();
        }
    }
}

impl OutputHandler for State {}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.globals.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.globals.xdg_shell_state
    }

    fn client_pong(&mut self, client: ShellClient) {
        self.pong(&client);
    }

    fn new_toplevel(&mut self, toplevel: ToplevelSurface) {
        let workspace = self.screens[self.pointer_screen()].workspaces.active_id();
        self.windows.push(Window::new(toplevel, workspace));
    }

    fn toplevel_destroyed(&mut self, toplevel: ToplevelSurface) {
        self.withdraw_toplevel(toplevel.wl_surface());
        if let Some(index) = self.window_index(toplevel.wl_surface())
            && self.windows[index].size.is_some()
        {
            self.remember_frame(index);
        }
        let workspace = self.window_index(toplevel.wl_surface()).map(|index| {
            self.close_window(index);
            self.windows.remove(index).workspace
        });
        self.needs_redraw = true;
        self.refocus_after_losing(toplevel.wl_surface());
        if let Some(workspace) = workspace {
            self.window_left_workspace(workspace);
        }
    }

    fn title_changed(&mut self, toplevel: ToplevelSurface) {
        self.publish_toplevel(toplevel.wl_surface());
        self.needs_redraw = true;
    }

    fn app_id_changed(&mut self, toplevel: ToplevelSurface) {
        self.recall_late(toplevel.wl_surface());
        self.publish_toplevel(toplevel.wl_surface());
        let key = self.key_window();
        self.announce_active_app(key.as_ref());
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        if let Configure::Toplevel(configure) = configure
            && let Some(index) = self.window_index(&surface)
        {
            self.windows[index].acked = Some(configure.serial);
        }
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: WlSeat, serial: Serial) {
        self.request_move(surface.wl_surface(), &seat, serial);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        self.request_resize(surface.wl_surface(), &seat, serial, edges);
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.maximize(surface.wl_surface());
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.unmaximize(surface.wl_surface());
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        self.fullscreen(surface.wl_surface());
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.unfullscreen(surface.wl_surface());
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        self.minimize(surface.wl_surface());
    }

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        State::new_popup(self, surface, positioner);
    }

    fn grab(&mut self, surface: PopupSurface, seat: WlSeat, serial: Serial) {
        self.grab_popup(surface, &seat, serial);
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        self.reposition_popup(surface, positioner, token);
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.needs_redraw = true;
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.globals.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        if let CursorImageStatus::Named(icon) = image {
            self.load_named_cursor(icon);
        }
        self.cursor_status = image;
        self.needs_redraw = true;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let client = focused.and_then(|surface| self.display_handle.get_client(surface.id()).ok());
        set_data_device_focus(&self.display_handle, seat, client.clone());
        set_primary_focus(&self.display_handle, seat, client);
        if let Some(rect) = focused
            .and_then(|surface| self.window_index(surface))
            .and_then(|index| self.windows[index].geometry())
        {
            self.zoom
                .follow((rect.location + rect.size.downscale(2).to_point()).to_f64());
        }
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.globals.primary_selection_state
    }
}

impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.globals.data_control_state
    }
}

delegate_compositor!(State);
delegate_data_device!(State);
delegate_dmabuf!(State);
delegate_output!(State);
delegate_presentation!(State);
delegate_ext_data_control!(State);
delegate_primary_selection!(State);
delegate_seat!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_layer_shell!(State);
delegate_xdg_activation!(State);
delegate_viewporter!(State);
delegate_fractional_scale!(State);
delegate_cursor_shape!(State);

impl TabletSeatHandler for State {}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.globals.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let screen = output
            .as_ref()
            .and_then(|output| {
                self.screens
                    .iter()
                    .position(|screen| screen.output.owns(output))
            })
            .unwrap_or_else(|| self.screen_for_layer(&namespace));
        let spans_the_edge = namespace == MENU_BAR_NAMESPACE;
        let mapped = MappedLayer::new(surface.clone(), namespace);
        let mut map = layer_map_for_output(&self.screens[screen].output);
        let arranged_after: Vec<MappedLayer> = if spans_the_edge {
            map.layers().cloned().collect()
        } else {
            Vec::new()
        };
        for layer in &arranged_after {
            map.unmap_layer(layer);
        }
        if map.map_layer(&mapped).is_err() {
            surface.send_close();
        }
        for layer in &arranged_after {
            if map.map_layer(layer).is_err() {
                layer.layer_surface().send_close();
            }
        }
    }

    fn new_popup(&mut self, _parent: LayerSurface, popup: PopupSurface) {
        self.adopt_layer_popup(popup);
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        if let Some(screen) = self.layer_screen(surface.wl_surface()) {
            let mut map = layer_map_for_output(&self.screens[screen].output);
            let mapped = map
                .layer_for_surface(surface.wl_surface(), WindowSurfaceType::TOPLEVEL)
                .cloned();
            if let Some(mapped) = mapped {
                map.unmap_layer(&mapped);
            }
        }
        self.needs_redraw = true;
        self.follow_panel_zones();
        self.follow_exclusive_layer(surface.wl_surface(), None);
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.globals.activation_state
    }

    fn token_created(&mut self, token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        let from_shell = data
            .client_id
            .as_ref()
            .is_some_and(|id| shell::is_shell_client(&self.display_handle, id));
        let valid = from_shell
            || match self.focused() {
                None => true,
                Some(focused) => focused.client().map(|client| client.id()) == data.client_id,
            };
        self.globals
            .activation_state
            .retain_tokens(|_, data| data.timestamp.elapsed() < ACTIVATION_TOKEN_LIFETIME);
        let known = &self.globals.activation_state;
        self.refused_tokens
            .retain(|refused| known.data_for_token(refused).is_some());
        if !valid {
            self.refused_tokens.push(token);
        }
        true
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        _token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let app_id = shell::toplevel_names(&surface).1;
        self.globals.activation_state.remove_token(&token);
        if let Some(index) = self
            .refused_tokens
            .iter()
            .position(|refused| refused == &token)
        {
            self.refused_tokens.remove(index);
            self.request_attention(app_id);
            return;
        }
        self.show_app(&app_id);
        self.unminimize(&surface);
        let mapped = self
            .windows
            .iter()
            .any(|window| window.surface() == &surface && window.visible());
        if mapped {
            self.focus(Some(surface));
        } else {
            self.pending_activation = Some(surface);
        }
    }
}

impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let scale = self.screens[self.screen_of_surface(&surface)].scale();
        send_preferred_scale(&surface, scale);
    }
}
