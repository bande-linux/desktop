use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Format as DmabufFormat, Fourcc, Modifier};
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::element::Kind as ElementKind;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::backend::renderer::element::memory::{
    MemoryRenderBufferRenderElement,
};
use smithay::desktop::{
    LayerSurface as MappedLayer, PopupGrab, PopupManager, WindowSurfaceType, layer_map_for_output,
};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus};
use smithay::render_elements;
use smithay::output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_callback::WlCallback;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle, Resource};
use smithay::utils::{Logical, Physical, Point, Raw, SERIAL_COUNTER, Serial, Size};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes,
    TraversalAction, with_states, with_surface_tree_downward,
};
use smithay::wayland::dmabuf::{
    DmabufFeedback, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier, get_dmabuf,
};
use smithay::wayland::fractional_scale::{
    FractionalScaleHandler, FractionalScaleManagerState, with_fractional_scale,
};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::presentation::{
    PresentationFeedbackCachedState, PresentationFeedbackCallback, PresentationState, Refresh,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{DataDeviceState, set_data_device_focus};
use smithay::wayland::selection::ext_data_control::{DataControlHandler, DataControlState};
use smithay::wayland::selection::primary_selection::{
    PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
};
use smithay::wayland::shell::xdg::{
    Configure, PopupSurface, PositionerState, ShellClient, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelListState;
use smithay::wayland::shell::wlr_layer::{
    KeyboardInteractivity, Layer, LayerSurface, LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState, with_buffer_contents};
use smithay::wayland::session_lock::SessionLockManagerState;
use smithay::wayland::shell::xdg::decoration::XdgDecorationState;
use smithay::wayland::viewporter::ViewporterState;
use smithay::wayland::xdg_activation::{
    XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
};
use smithay::{
    delegate_compositor, delegate_cursor_shape, delegate_data_device, delegate_dmabuf, delegate_ext_data_control,
    delegate_fractional_scale, delegate_layer_shell, delegate_output, delegate_presentation,
    delegate_primary_selection, delegate_seat, delegate_shm, delegate_viewporter,
    delegate_xdg_activation, delegate_xdg_shell,
};
use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use unbound_protocols::server::unbound_app_v1::UnboundAppV1;
use unbound_protocols::server::unbound_shell_v1::UnboundShellV1;
use unbound_render::VulkanRenderer;

use crate::clock::Vblank;
use crate::cursor::{self, Cursor, CursorTheme};
use crate::lock::Lock;
use crate::motion::IconRect;
use crate::scene::{
    BlurElement, Closing, Drawn, FillElement, PieceElement, ShadowElement, WindowElement,
    preview_elements, surface_blurs, tie_blurs_to_what_lies_below, window_elements,
};
use crate::session::{ForceQuit, LockScreen, SessionClient, SessionRole};
use crate::shell;
use crate::tile::{Preview, SnapDrag};
use crate::window::Window;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::input_method::InputMethodManagerState;
use smithay::wayland::pointer_gestures::PointerGesturesState;
use smithay::wayland::tablet_manager::TabletSeatHandler;
use smithay::wayland::text_input::TextInputManagerState;
use smithay::wayland::virtual_keyboard::VirtualKeyboardManagerState;

pub const REFRESH: Duration = Duration::from_nanos(1_000_000_000 / 120);

pub struct ScanoutDevice {
    pub node: DrmNode,
    pub feedback: DmabufFeedback,
}

pub struct OutputConfig {
    pub name: String,
    pub model: String,
    pub size: Size<i32, Physical>,
    pub physical_size_mm: Size<i32, Raw>,
    pub refresh: Duration,
    pub scanout: Option<ScanoutDevice>,
}

const REFERENCE_PIXELS_PER_INCH: f64 = 110.0;
const SCALE_STEP: f64 = 0.25;
const HIGH_DENSITY_HEIGHT: i32 = 1600;

pub fn automatic_scale(size: Size<i32, Physical>, physical_size_mm: Size<i32, Raw>) -> f64 {
    if physical_size_mm.w > 0 && physical_size_mm.h > 0 {
        let pixels_per_inch = f64::from(size.w) / (f64::from(physical_size_mm.w) / 25.4);
        let scale = (pixels_per_inch / REFERENCE_PIXELS_PER_INCH / SCALE_STEP).round() * SCALE_STEP;
        return scale.clamp(1.0, 3.0);
    }
    if size.h >= HIGH_DENSITY_HEIGHT {
        2.0
    } else {
        1.0
    }
}

#[derive(Clone)]
pub(crate) enum Placed<'a> {
    Surface(WlSurface, Point<i32, Logical>),
    Window(&'a Window),
    Closing(&'a Closing),
}

type Composed = (
    Vec<OutputElement>,
    Vec<(WlSurface, Option<Drawn>)>,
    Option<(Vec<crate::scene::Piece>, crate::scene::Area)>,
);

pub(crate) struct ExclusiveLayer {
    pub(crate) surface: WlSurface,
    pub(crate) previous: Option<WlSurface>,
}

pub struct State {
    pub(crate) display_handle: DisplayHandle,
    pub(crate) syncobj_state: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,
    pub(crate) blockers: smithay::reexports::calloop::LoopHandle<'static, State>,
    pub(crate) output_size: Size<i32, Physical>,
    pub(crate) output_scale: f64,
    pub(crate) refresh: Duration,
    scanout_node: Option<DrmNode>,
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    pub(crate) output: Output,
    _output_manager_state: OutputManagerState,
    _presentation_state: PresentationState,
    layer_shell_state: WlrLayerShellState,
    activation_state: XdgActivationState,
    _viewporter_state: ViewporterState,
    _fractional_scale_state: FractionalScaleManagerState,
    _cursor_shape_state: CursorShapeManagerState,
    _pointer_gestures_state: PointerGesturesState,
    _decoration_state: XdgDecorationState,
    _text_input_state: TextInputManagerState,
    _input_method_state: InputMethodManagerState,
    _virtual_keyboard_state: VirtualKeyboardManagerState,
    pending_activation: Option<WlSurface>,
    pub(crate) shell_resources: Vec<UnboundShellV1>,
    pub(crate) app_resources: Vec<UnboundAppV1>,
    pub(crate) foreign_toplevel_state: ForeignToplevelListState,
    pub(crate) exclusive_layer: Option<ExclusiveLayer>,
    exclusive_candidates: Vec<(WlSurface, Layer)>,
    seat_state: SeatState<State>,
    pub(crate) seat: Seat<State>,
    pub(crate) data_device_state: DataDeviceState,
    primary_selection_state: PrimarySelectionState,
    data_control_state: DataControlState,
    pub(crate) windows: Vec<Window>,
    pub(crate) popups: PopupManager,
    pub(crate) lock_screen_requested: bool,
    pub(crate) force_quit_requested: bool,
    pub(crate) refused_tokens: Vec<XdgActivationToken>,
    pub(crate) drag: Option<crate::drag::Drag>,
    pub(crate) slide_back: Option<crate::scene::SlideBack>,
    pub(crate) watchdog: crate::watchdog::Watchdog,
    pub(crate) desktop: crate::gesture::ShowDesktop,
    pub(crate) workspaces: crate::workspace::Workspaces,
    pub(crate) gesture_route: Option<crate::gesture::Route>,
    pub(crate) active_app: String,
    pub(crate) idle: crate::idle::Idle,
    _idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    pub(crate) lock_states: Option<std::sync::mpsc::Sender<bool>>,
    pub(crate) lock: Option<Lock>,
    pub(crate) session_lock_state: SessionLockManagerState,
    pub(crate) popup_grab: Option<PopupGrab<State>>,
    pub(crate) swallowed_button: Option<u32>,
    pub(crate) minimize_sequence: u64,
    pub(crate) closing: Vec<Closing>,
    pub(crate) icon_rects: Vec<IconRect>,
    pub(crate) snap: Option<SnapDrag>,
    pub(crate) preview: Option<Preview>,
    pub(crate) reduce_motion: bool,
    pub(crate) needs_redraw: bool,
    pub(crate) pointer_location: Point<f64, Logical>,
    pub(crate) cursor_shown: bool,
    cursor_status: CursorImageStatus,
    cursor_theme: CursorTheme,
    cursor_arrow: Cursor,
    named_cursors: HashMap<CursorIcon, Option<Cursor>>,
    pub(crate) held_shortcut_keys: Vec<u32>,
    pub(crate) input_devices: usize,
    pub(crate) pending_captures: Vec<crate::capture::PendingCapture>,
}

pub struct Headless {
    display: Display<State>,
    pub(crate) fences: smithay::reexports::calloop::EventLoop<'static, State>,
    pub(crate) state: State,
    latched_feedback: Vec<PresentationFeedbackCallback>,
    latched_frame_callbacks: Vec<WlCallback>,
    pub(crate) session_clients: Vec<SessionClient>,
    pub(crate) lock_screen: LockScreen,
    pub(crate) force_quit: ForceQuit,
    pub(crate) socket_name: Option<OsString>,
}

impl Headless {
    pub fn new(output_size: Size<i32, Physical>) -> io::Result<Self> {
        Self::with_output(OutputConfig {
            name: "HEADLESS-1".to_owned(),
            model: "Headless".to_owned(),
            size: output_size,
            physical_size_mm: (0, 0).into(),
            refresh: REFRESH,
            scanout: None,
        })
    }

    pub fn with_output(config: OutputConfig) -> io::Result<Self> {
        let output_size = config.size;
        let display = Display::new().map_err(io::Error::other)?;
        let fences = smithay::reexports::calloop::EventLoop::try_new().map_err(io::Error::other)?;
        let handle = display.handle();
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&handle, "seat0");
        seat.add_keyboard(XkbConfig::default(), 600, 25)
            .map_err(io::Error::other)?;
        seat.add_pointer();
        crate::idle::create_global(&handle);
        let primary_selection_state = PrimarySelectionState::new::<State>(&handle);
        let data_control_state =
            DataControlState::new::<State, _>(&handle, Some(&primary_selection_state), |client| {
                client
                    .get_data::<ClientState>()
                    .is_some_and(|state| state.role == Some(SessionRole::ClipboardDaemon))
            });
        let mut dmabuf_state = DmabufState::new();
        let dmabuf_global = match &config.scanout {
            Some(scanout) => dmabuf_state
                .create_global_with_default_feedback::<State>(&handle, &scanout.feedback),
            None => dmabuf_state.create_global::<State>(
                &handle,
                DMABUF_FOURCCS.map(|code| DmabufFormat {
                    code,
                    modifier: Modifier::Linear,
                }),
            ),
        };
        let output = Output::new(
            config.name,
            PhysicalProperties {
                size: config.physical_size_mm,
                subpixel: Subpixel::Unknown,
                make: "Unbound".to_owned(),
                model: config.model,
            },
        );
        let millihertz = 1_000_000_000_000u128 / config.refresh.as_nanos().max(1);
        let mode = Mode {
            size: (output_size.w, output_size.h).into(),
            refresh: i32::try_from(millihertz).unwrap_or(i32::MAX),
        };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        output.create_global::<State>(&handle);
        handle.create_global::<State, UnboundShellV1, ()>(9, ());
        handle.create_global::<State, UnboundAppV1, ()>(3, ());
        handle.create_global::<State, ExtBackgroundEffectManagerV1, ()>(1, ());
        State::create_capture_globals(&handle);
        let state = State {
            display_handle: handle.clone(),
            syncobj_state: None,
            blockers: fences.handle(),
            output_size,
            output_scale: 1.0,
            refresh: config.refresh,
            scanout_node: config.scanout.as_ref().map(|scanout| scanout.node),
            compositor_state: CompositorState::new::<State>(&handle),
            xdg_shell_state: XdgShellState::new_with_capabilities::<State>(
                &handle,
                [
                    xdg_toplevel::WmCapabilities::Maximize,
                    xdg_toplevel::WmCapabilities::Fullscreen,
                    xdg_toplevel::WmCapabilities::Minimize,
                ],
            ),
            shm_state: ShmState::new::<State>(&handle, []),
            dmabuf_state,
            _dmabuf_global: dmabuf_global,
            output,
            _output_manager_state: OutputManagerState::new(),
            _presentation_state: PresentationState::new::<State>(&handle, MONOTONIC),
            layer_shell_state: WlrLayerShellState::new::<State>(&handle),
            activation_state: XdgActivationState::new::<State>(&handle),
            _viewporter_state: ViewporterState::new::<State>(&handle),
            _fractional_scale_state: FractionalScaleManagerState::new::<State>(&handle),
            _decoration_state: XdgDecorationState::new::<State>(&handle),
            _cursor_shape_state: CursorShapeManagerState::new::<State>(&handle),
            _pointer_gestures_state: PointerGesturesState::new::<State>(&handle),
            _text_input_state: TextInputManagerState::new::<State>(&handle),
            _input_method_state: InputMethodManagerState::new::<State, _>(&handle, |_| true),
            _virtual_keyboard_state: VirtualKeyboardManagerState::new::<State, _>(&handle, |_| {
                true
            }),
            pending_activation: None,
            shell_resources: Vec::new(),
            app_resources: Vec::new(),
            foreign_toplevel_state: ForeignToplevelListState::new_with_filter::<State>(
                &handle,
                shell::is_shell,
            ),
            exclusive_layer: None,
            exclusive_candidates: Vec::new(),
            seat_state,
            seat,
            data_device_state: DataDeviceState::new::<State>(&handle),
            primary_selection_state,
            data_control_state,
            windows: Vec::new(),
            popups: PopupManager::default(),
            lock_screen_requested: false,
            force_quit_requested: false,
            refused_tokens: Vec::new(),
            drag: None,
            slide_back: None,
            watchdog: crate::watchdog::Watchdog::default(),
            desktop: crate::gesture::ShowDesktop::default(),
            workspaces: crate::workspace::Workspaces::default(),
            gesture_route: None,
            active_app: String::new(),
            idle: crate::idle::Idle::default(),
            _idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<
                State,
            >(&handle),
            lock_states: None,
            lock: None,
            session_lock_state: SessionLockManagerState::new::<State, _>(&handle, shell::is_shell),
            popup_grab: None,
            swallowed_button: None,
            minimize_sequence: 0,
            closing: Vec::new(),
            icon_rects: Vec::new(),
            snap: None,
            preview: None,
            reduce_motion: false,
            needs_redraw: false,
            pointer_location: (
                f64::from(output_size.w) / 2.0,
                f64::from(output_size.h) / 2.0,
            )
                .into(),
            cursor_shown: false,
            cursor_status: CursorImageStatus::default_named(),
            cursor_theme: CursorTheme::builtin(),
            cursor_arrow: Cursor {
                buffer: cursor::arrow(),
                hotspot: Point::default(),
            },
            named_cursors: HashMap::new(),
            held_shortcut_keys: Vec::new(),
            input_devices: 0,
            pending_captures: Vec::new(),
        };
        Ok(Self {
            display,
            fences,
            state,
            latched_feedback: Vec::new(),
            latched_frame_callbacks: Vec::new(),
            session_clients: Vec::new(),
            lock_screen: LockScreen::new(),
            force_quit: ForceQuit::new(),
            socket_name: None,
        })
    }

    pub fn insert_client(&mut self, stream: UnixStream) -> io::Result<Client> {
        self.display
            .handle()
            .insert_client(stream, Arc::new(ClientState::default()))
    }

    pub(crate) fn display_handle(&self) -> DisplayHandle {
        self.display.handle()
    }

    pub fn dispatch(&mut self) -> io::Result<()> {
        self.clear_signalled_fences()?;
        self.display.dispatch_clients(&mut self.state)?;
        self.state.popups.cleanup();
        self.state.refocus_pointer();
        self.display.flush_clients()
    }

    pub fn poll_fd(&mut self) -> io::Result<OwnedFd> {
        self.display.backend().poll_fd().try_clone_to_owned()
    }

    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.state.needs_redraw)
    }

    pub fn latch(&mut self) {
        self.state.latch_lock();
        let unmapped = self
            .state
            .windows
            .iter()
            .filter(|window| window.size.is_none())
            .map(|window| window.surface().clone());
        let roots = self
            .state
            .stack()
            .into_iter()
            .map(|(surface, _)| surface)
            .chain(unmapped)
            .collect::<Vec<_>>();
        for root in roots {
            with_surface_tree_downward(
                &root,
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |_, states, _| {
                    self.latched_feedback.append(
                        &mut states
                            .cached_state
                            .get::<PresentationFeedbackCachedState>()
                            .current()
                            .callbacks,
                    );
                    self.latched_frame_callbacks.append(
                        &mut states
                            .cached_state
                            .get::<SurfaceAttributes>()
                            .current()
                            .frame_callbacks,
                    );
                },
                |_, _, _| true,
            );
        }
    }

    pub fn present(&mut self, vblank: Vblank) -> io::Result<()> {
        for feedback in self.latched_feedback.drain(..) {
            feedback.presented(
                &self.state.output,
                vblank.time,
                Refresh::fixed(self.state.refresh),
                vblank.sequence,
                Kind::Vsync,
            );
        }
        let millis = (vblank.time.as_millis() % (1 << 32)) as u32;
        for callback in self.latched_frame_callbacks.drain(..) {
            callback.done(millis);
        }
        self.state.confirm_lock();
        self.display.flush_clients()
    }

    pub fn output_size(&self) -> Size<i32, Physical> {
        self.state.output_size
    }

    pub fn output_scale(&self) -> f64 {
        self.state.output_scale
    }

    pub fn logical_output_size(&self) -> Size<i32, Logical> {
        self.state.logical_output_size()
    }

    pub fn set_output_scale(&mut self, scale: f64) {
        self.state.set_output_scale(scale);
    }

    pub fn windows(&self) -> &[Window] {
        &self.state.windows
    }

    pub fn render_elements(&mut self, renderer: &mut VulkanRenderer) -> Vec<OutputElement> {
        let (elements, drawn, icon) = self.compose(renderer, true);
        self.state.remember_drag_icon(icon);
        for window in &mut self.state.windows {
            window.drawn = None;
        }
        for (surface, drawn) in drawn {
            if let Some(index) = self.state.window_index(&surface) {
                self.state.windows[index].drawn = drawn;
            }
        }
        elements
    }

    pub(crate) fn scene_elements(
        &self,
        renderer: &mut VulkanRenderer,
        with_cursor: bool,
    ) -> Vec<OutputElement> {
        self.compose(renderer, with_cursor).0
    }

    fn compose(&self, renderer: &mut VulkanRenderer, with_cursor: bool) -> Composed {
        let mut drawn = Vec::new();
        let (mut elements, icon): (Vec<OutputElement>, _) = if with_cursor {
            let (dragged, icon) = self.state.drag_icon_elements(renderer);
            let mut elements = self.cursor_elements(renderer);
            elements.extend(dragged);
            (elements, icon)
        } else {
            (Vec::new(), None)
        };
        let scale = self.state.output_scale;
        let key = self.state.key_window();
        for placed in self.state.scene(true) {
            match placed {
                Placed::Surface(surface, location) => {
                    elements.extend(render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        location.to_f64().to_physical(scale).to_i32_round::<i32>(),
                        scale,
                        1.0,
                        ElementKind::Unspecified,
                    ));
                    elements.extend(surface_blurs(&surface, location, scale));
                }
                Placed::Window(window) => {
                    let active = key.as_ref() == Some(window.surface());
                    let hung = window
                        .surface()
                        .client()
                        .is_some_and(|owner| self.state.watchdog.is_unresponsive(&owner.id()));
                    let presented = self.state.presented(window);
                    let (shown, look) =
                        window_elements(window, renderer, scale, active, hung, presented);
                    elements.extend(shown);
                    drawn.push((window.surface().clone(), look));
                    if let Some(preview) = &self.state.preview
                        && &preview.owner == window.surface()
                    {
                        elements.extend(preview_elements(preview, scale));
                    }
                }
                Placed::Closing(closing) => elements.extend(closing.render_elements(scale)),
            }
        }
        tie_blurs_to_what_lies_below(&mut elements, scale);
        (elements, drawn, icon)
    }

    pub fn popup_count(&self) -> usize {
        self.state
            .stack()
            .iter()
            .filter(|(surface, _)| {
                self.state.popups.find_popup(surface).is_some()
                    && with_renderer_surface_state(surface, |state| state.buffer().is_some())
                        .unwrap_or(false)
            })
            .count()
    }

    pub fn use_cursor_theme(&mut self, theme: CursorTheme) {
        self.state.use_cursor_theme(theme);
    }

    pub fn cursor_theme(&self) -> &CursorTheme {
        &self.state.cursor_theme
    }

    pub fn usable_area(&self) -> Rect {
        self.state.usable_area()
    }

    fn cursor_elements(&self, renderer: &mut VulkanRenderer) -> Vec<OutputElement> {
        let state = &self.state;
        if !state.cursor_shown {
            return Vec::new();
        }
        let scale = state.output_scale;
        let at: Point<i32, Physical> = state.pointer_location.to_physical(scale).to_i32_round();
        let busy = CursorImageStatus::Named(CursorIcon::Wait);
        let over_hung_app = state
            .seat
            .get_pointer()
            .and_then(|pointer| pointer.current_focus())
            .and_then(|surface| surface.client())
            .is_some_and(|owner| state.watchdog.is_unresponsive(&owner.id()));
        let status = if over_hung_app {
            &busy
        } else {
            &state.cursor_status
        };
        match status {
            CursorImageStatus::Hidden => Vec::new(),
            CursorImageStatus::Surface(surface) => {
                let hotspot = with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .and_then(|attributes| attributes.lock().ok().map(|a| a.hotspot))
                        .unwrap_or_default()
                });
                let hotspot = hotspot.to_f64().to_physical(scale).to_i32_round::<i32>();
                render_elements_from_surface_tree(
                    renderer,
                    surface,
                    (at.x - hotspot.x, at.y - hotspot.y),
                    scale,
                    1.0,
                    ElementKind::Cursor,
                )
            }
            CursorImageStatus::Named(icon) => {
                let cursor = state
                    .named_cursors
                    .get(icon)
                    .and_then(Option::as_ref)
                    .unwrap_or(&state.cursor_arrow);
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    (
                        f64::from(at.x - cursor.hotspot.x),
                        f64::from(at.y - cursor.hotspot.y),
                    ),
                    &cursor.buffer,
                    None,
                    None,
                    None,
                    ElementKind::Cursor,
                )
                .map(OutputElement::Cursor)
                .into_iter()
                .collect()
            }
        }
    }

    pub(crate) fn flush(&mut self) -> io::Result<()> {
        self.display.flush_clients()
    }
}

#[derive(Default)]
pub(crate) struct ClientState {
    pub(crate) compositor_state: CompositorClientState,
    pub(crate) role: Option<SessionRole>,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

const MONOTONIC: u32 = 1;

const DMABUF_FOURCCS: [Fourcc; 2] = [Fourcc::Argb8888, Fourcc::Xrgb8888];

pub(crate) fn mapped_size(surface: &WlSurface) -> Option<Size<i32, Logical>> {
    with_renderer_surface_state(surface, |state| {
        let buffer = state.buffer()?;
        buffer_size(buffer)?;
        state.surface_size()
    })
    .flatten()
}

fn send_preferred_scale(surface: &WlSurface, scale: f64) {
    with_surface_tree_downward(
        surface,
        (),
        |_, _, ()| TraversalAction::DoChildren(()),
        |_, states, ()| {
            with_fractional_scale(states, |fractional| fractional.set_preferred_scale(scale));
        },
        |_, _, ()| true,
    );
}

fn buffer_size(buffer: &WlBuffer) -> Option<Size<i32, Physical>> {
    match get_dmabuf(buffer) {
        Ok(dmabuf) => Some(Size::from((dmabuf.size().w, dmabuf.size().h))),
        Err(_) => shm_size(buffer),
    }
}

fn shm_size(buffer: &WlBuffer) -> Option<Size<i32, Physical>> {
    let data = with_buffer_contents(buffer, |_, _, data| data).ok()?;
    matches!(data.format, Format::Argb8888 | Format::Xrgb8888)
        .then(|| Size::from((data.width, data.height)))
}

impl State {
    pub(crate) fn load_named_cursor(&mut self, icon: CursorIcon) {
        let theme = &self.cursor_theme;
        let scale = self.output_scale;
        self.named_cursors
            .entry(icon)
            .or_insert_with(|| theme.load(icon, scale));
    }

    pub(crate) fn use_cursor_theme(&mut self, theme: CursorTheme) {
        self.cursor_theme = theme;
        self.named_cursors.clear();
        if let CursorImageStatus::Named(icon) = self.cursor_status {
            self.load_named_cursor(icon);
        }
        self.needs_redraw = true;
    }

    pub(crate) fn focused(&self) -> Option<WlSurface> {
        self.seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus())
    }

    pub(crate) fn focus(&mut self, surface: Option<WlSurface>) {
        if self.redirect_focus_while_locked(&surface) {
            return;
        }
        if let Some(surface) = &surface
            && self.window_index(surface).is_some()
        {
            self.hide_desktop();
            self.show_workspace_of(surface);
        }
        if let Some(exclusive) = &mut self.exclusive_layer
            && surface.as_ref() != Some(&exclusive.surface)
        {
            exclusive.previous = surface.clone();
            if let Some(surface) = &surface
                && let Some(index) = self
                    .windows
                    .iter()
                    .position(|window| window.toplevel.wl_surface() == surface)
            {
                let window = self.windows.remove(index);
                self.windows.push(window);
                self.needs_redraw = true;
            }
            self.sync_activated();
            return;
        }
        if let Some(surface) = &surface
            && let Some(index) = self
                .windows
                .iter()
                .position(|window| window.toplevel.wl_surface() == surface)
        {
            let window = self.windows.remove(index);
            self.windows.push(window);
            self.needs_redraw = true;
        }
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
        self.sync_activated();
    }

    pub(crate) fn frontmost(&self, owner: Option<&Client>) -> Option<WlSurface> {
        self.windows
            .iter()
            .rev()
            .filter(|window| self.present(window))
            .map(|window| window.toplevel.wl_surface())
            .find(|surface| {
                surface
                    .client()
                    .is_some_and(|client| owner.is_none_or(|owner| &client == owner))
            })
            .cloned()
    }

    pub(crate) fn refocus_after_losing(&mut self, lost: &WlSurface) {
        if self.focused().as_ref() != Some(lost) {
            return;
        }
        let next = match lost.client() {
            Some(owner) => self.frontmost(Some(&owner)),
            None => self.frontmost(None),
        };
        self.focus(next);
    }

    fn takes_focus_when_mapped(&self, surface: &WlSurface) -> bool {
        let Some(focused) = self.focused() else {
            return true;
        };
        let owner = surface.client();
        if focused.client() == owner {
            return true;
        }
        !self.windows.iter().any(|window| {
            window.size.is_some()
                && window.toplevel.wl_surface() != surface
                && window.toplevel.wl_surface().client() == owner
        })
    }

    fn place_behind_frontmost(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .windows
            .iter()
            .position(|window| window.toplevel.wl_surface() == surface)
        else {
            return;
        };
        let window = self.windows.remove(index);
        let behind = self.windows.len().saturating_sub(1);
        self.windows.insert(behind, window);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub location: Point<i32, Logical>,
    pub size: Size<i32, Logical>,
}

pub(crate) fn centered(area: Rect, window: Size<i32, Logical>) -> Point<i32, Logical> {
    (
        area.location.x + (area.size.w - window.w) / 2,
        area.location.y + (area.size.h - window.h) / 2,
    )
        .into()
}

impl State {
    pub(crate) fn usable_area(&self) -> Rect {
        let zone = layer_map_for_output(&self.output).non_exclusive_zone();
        Rect {
            location: (zone.loc.x, zone.loc.y).into(),
            size: (zone.size.w, zone.size.h).into(),
        }
    }

    pub(crate) fn logical_output_size(&self) -> Size<i32, Logical> {
        self.output_size
            .to_f64()
            .to_logical(self.output_scale)
            .to_i32_round()
    }

    pub(crate) fn set_output_scale(&mut self, scale: f64) {
        self.output_scale = scale;
        self.output
            .change_current_state(None, None, Some(OutputScale::Fractional(scale)), None);
        let center = self.logical_output_size().to_f64();
        self.pointer_location = (center.w / 2.0, center.h / 2.0).into();
        self.named_cursors.clear();
        if let CursorImageStatus::Named(icon) = self.cursor_status {
            self.load_named_cursor(icon);
        }
        layer_map_for_output(&self.output).arrange();
        for (surface, _) in self.stack() {
            send_preferred_scale(&surface, scale);
        }
        self.needs_redraw = true;
    }

    pub(crate) fn placed_layers(&self, layer: Layer) -> Vec<(WlSurface, Point<i32, Logical>)> {
        let map = layer_map_for_output(&self.output);
        map.layers_on(layer)
            .rev()
            .filter_map(|mapped| {
                let geometry = map.layer_geometry(mapped)?;
                Some((
                    mapped.wl_surface().clone(),
                    (geometry.loc.x, geometry.loc.y).into(),
                ))
            })
            .collect()
    }

    pub(crate) fn layer_origin(&self, surface: &WlSurface) -> Option<Point<i32, Logical>> {
        let map = layer_map_for_output(&self.output);
        let layer = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)?;
        let geometry = map.layer_geometry(layer)?;
        Some((geometry.loc.x, geometry.loc.y).into())
    }

    pub(crate) fn stack(&self) -> Vec<(WlSurface, Point<i32, Logical>)> {
        self.scene(false)
            .into_iter()
            .filter_map(|placed| match placed {
                Placed::Surface(surface, origin) => Some((surface, origin)),
                Placed::Window(window) => Some((window.surface().clone(), window.surface_origin())),
                Placed::Closing(_) => None,
            })
            .collect()
    }

    pub(crate) fn scene(&self, rendered: bool) -> Vec<Placed<'_>> {
        if self.is_locked() {
            return self
                .locked_stack()
                .into_iter()
                .map(|(surface, origin)| Placed::Surface(surface, origin))
                .collect();
        }
        let popups = |surface: &WlSurface, geometry_origin| {
            self.popup_surfaces(surface, geometry_origin)
                .into_iter()
                .map(|(surface, origin)| Placed::Surface(surface, origin))
        };
        let layers = |layer: Layer| {
            self.placed_layers(layer)
                .into_iter()
                .flat_map(|(surface, at)| {
                    popups(&surface, at).chain([Placed::Surface(surface, at)])
                })
                .collect::<Vec<_>>()
        };
        let visible = self
            .windows
            .iter()
            .rev()
            .filter(|window| {
                if rendered {
                    window.rendered() && self.on_view(window)
                } else {
                    self.present(window) && !self.desktop.hides_windows()
                }
            })
            .collect::<Vec<_>>();
        let (front, rest): (Vec<&Window>, Vec<&Window>) = visible
            .into_iter()
            .partition(|window| self.on_full_screen_space(window));
        let (leaving, rest): (Vec<&Window>, Vec<&Window>) =
            rest.into_iter().partition(|window| window.is_minimized());
        [
            layers(Layer::Overlay),
            self.window_entries(&leaving),
            self.window_entries(&front),
            layers(Layer::Top),
            self.closing
                .iter()
                .filter(|_| rendered)
                .map(Placed::Closing)
                .collect(),
            self.window_entries(&rest),
            layers(Layer::Bottom),
            layers(Layer::Background),
        ]
        .concat()
    }

    fn window_entries<'a>(&'a self, windows: &[&'a Window]) -> Vec<Placed<'a>> {
        windows
            .iter()
            .flat_map(|window| {
                self.popup_surfaces(window.surface(), window.location)
                    .into_iter()
                    .map(|(surface, origin)| Placed::Surface(surface, origin))
                    .chain([Placed::Window(window)])
            })
            .collect()
    }

    fn commit_layer(&mut self, surface: &WlSurface) -> bool {
        let mut map = layer_map_for_output(&self.output);
        let Some(layer) = map
            .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
            .cloned()
        else {
            return false;
        };
        map.arrange();
        let mapped =
            with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false);
        let exclusive = (mapped
            && matches!(layer.layer(), Layer::Top | Layer::Overlay)
            && layer.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive)
            .then(|| layer.layer());
        drop(map);
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

    fn follow_exclusive_layer(&mut self, surface: &WlSurface, exclusive: Option<Layer>) {
        let known = self
            .exclusive_candidates
            .iter()
            .position(|(candidate, _)| candidate == surface);
        match (exclusive, known) {
            (Some(layer), Some(index)) => self.exclusive_candidates[index].1 = layer,
            (Some(layer), None) => self.exclusive_candidates.push((surface.clone(), layer)),
            (None, Some(index)) => {
                self.exclusive_candidates.remove(index);
            }
            (None, None) => return,
        }
        let holder = self
            .exclusive_candidates
            .iter()
            .max_by_key(|(_, layer)| *layer == Layer::Overlay)
            .map(|(candidate, _)| candidate.clone());
        if holder.as_ref() == self.exclusive_layer.as_ref().map(|layer| &layer.surface) {
            return;
        }
        let Some(holder) = holder else {
            self.release_exclusive_layer();
            return;
        };
        let previous = match self.exclusive_layer.take() {
            Some(layer) => layer.previous,
            None => self.focused(),
        };
        self.focus(Some(holder.clone()));
        self.exclusive_layer = Some(ExclusiveLayer {
            surface: holder,
            previous,
        });
        self.sync_activated();
    }

    fn release_exclusive_layer(&mut self) {
        let Some(layer) = self.exclusive_layer.take() else {
            return;
        };
        let previous = layer.previous.filter(|previous| {
            self.windows
                .iter()
                .any(|window| window.surface() == previous && self.present(window))
        });
        let next = previous.or_else(|| self.frontmost(None));
        self.focus(next);
    }
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        crate::sync::wait_for_fences(surface);
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        match client.get_data::<ClientState>() {
            Some(state) => &state.compositor_state,
            None => unreachable!("every client is inserted with ClientState"),
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

impl State {
    pub(crate) fn after_window_commit(
        &mut self,
        surface: &WlSurface,
        was_mapped: bool,
        mapped: bool,
    ) {
        match (was_mapped, mapped) {
            (false, true)
                if self.pending_activation.as_ref() == Some(surface)
                    || self.takes_focus_when_mapped(surface) =>
            {
                self.pending_activation = None;
                self.focus(Some(surface.clone()));
            }
            (false, true) => self.place_behind_frontmost(surface),
            (true, false) => {
                if let Some(index) = self.window_index(surface) {
                    self.close_window(index);
                }
                self.refocus_after_losing(surface);
                if let Some(index) = self.window_index(surface) {
                    self.window_left_workspace(self.windows[index].workspace);
                }
            }
            _ => {}
        }
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
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
        &self.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn client_pong(&mut self, client: ShellClient) {
        self.pong(&client);
    }

    fn new_toplevel(&mut self, toplevel: ToplevelSurface) {
        let workspace = self.workspaces.active_id();
        self.windows.push(Window::new(toplevel, workspace));
    }

    fn toplevel_destroyed(&mut self, toplevel: ToplevelSurface) {
        self.withdraw_toplevel(toplevel.wl_surface());
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
    }

    fn app_id_changed(&mut self, toplevel: ToplevelSurface) {
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

render_elements! {
    pub OutputElement<=VulkanRenderer>;
    Surface=WaylandSurfaceRenderElement<VulkanRenderer>,
    Window=WindowElement<WaylandSurfaceRenderElement<VulkanRenderer>>,
    Shadow=ShadowElement,
    Closing=WindowElement<PieceElement>,
    Fill=FillElement,
    Blur=BlurElement,
    Cursor=MemoryRenderBufferRenderElement<VulkanRenderer>,
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
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
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

impl DataControlHandler for State {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
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
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: LayerSurface,
        _output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let mapped = MappedLayer::new(surface.clone(), namespace);
        if layer_map_for_output(&self.output)
            .map_layer(&mapped)
            .is_err()
        {
            surface.send_close();
        }
    }

    fn layer_destroyed(&mut self, surface: LayerSurface) {
        let mut map = layer_map_for_output(&self.output);
        let mapped = map
            .layer_for_surface(surface.wl_surface(), WindowSurfaceType::TOPLEVEL)
            .cloned();
        if let Some(mapped) = mapped {
            map.unmap_layer(&mapped);
        }
        drop(map);
        self.needs_redraw = true;
        self.follow_exclusive_layer(surface.wl_surface(), None);
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation_state
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
        send_preferred_scale(&surface, self.output_scale);
    }
}
