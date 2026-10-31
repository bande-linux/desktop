use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::element::ScreenElement;
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Format as DmabufFormat, Fourcc, Modifier};
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::element::Kind as ElementKind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state};
use smithay::desktop::{
    LayerSurface as MappedLayer, PopupGrab, PopupManager, WindowSurfaceType, layer_map_for_output,
};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus};
use smithay::render_elements;

use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle, Resource};
use smithay::utils::{Logical, Physical, Point, SERIAL_COUNTER, Serial, Size, Transform};
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
    PresentationFeedbackCachedState, PresentationState, Refresh,
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
use smithay::wayland::shell::xdg::dialog::XdgDialogState;
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
use unbound_protocols::server::unbound_app_control_v1::UnboundAppControlV1;
use unbound_protocols::server::unbound_app_v1::UnboundAppV1;
use unbound_protocols::server::unbound_shell_v1::UnboundShellV1;
use unbound_render::VulkanRenderer;

use crate::clock::Vblank;
use crate::cursor::{self, Cursor, CursorTheme};
use crate::lock::Lock;
use crate::motion::IconRect;
use crate::scene::{
    BlurElement, Closing, Drawn, FillElement, PieceElement, Shading, ShadowElement, WindowElement,
    preview_elements, scrim, surface_blurs, tie_blurs_to_what_lies_below, window_elements,
};
use crate::screen::{OutputConfig, OutputSettings, Screen};
use crate::session::{LockScreen, SessionClient, SessionRole, Summoned};
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

pub(crate) const MENU_BAR_NAMESPACE: &str = "menubar";
pub(crate) const MENU_SEARCH_NAMESPACE: &str = "menu-search";

#[derive(Clone)]
pub(crate) enum Placed<'a> {
    Surface(WlSurface, Point<i32, Logical>),
    Window(&'a Window),
    Closing(&'a Closing),
    Overview(usize),
    Backdrop(usize),
    Scrim(WlSurface, usize),
}

type Composed<E = ScreenElement> = (
    Vec<E>,
    Vec<(WlSurface, Option<Drawn>)>,
    Option<(Vec<crate::scene::Piece>, crate::scene::Area)>,
);

fn relocated(elements: Vec<OutputElement>, by: Point<i32, Physical>) -> Vec<ScreenElement> {
    elements
        .into_iter()
        .map(|element| ScreenElement::new(element, by))
        .collect()
}

pub(crate) struct ExclusiveLayer {
    pub(crate) surface: WlSurface,
    pub(crate) previous: Option<WlSurface>,
}

pub struct State {
    pub(crate) display_handle: DisplayHandle,
    pub(crate) syncobj_state: Option<smithay::wayland::drm_syncobj::DrmSyncobjState>,
    pub(crate) blockers: smithay::reexports::calloop::LoopHandle<'static, State>,
    pub(crate) screens: Vec<Screen>,
    pub(crate) departed: Vec<crate::displays::Departed>,
    pub(crate) display_config: unbound_config::Config,
    pub(crate) night_light: crate::night::NightLight,
    pub(crate) output_managers: Vec<crate::management::Manager>,
    pub(crate) output_serial: u32,
    pub(crate) announced_workspaces: (usize, usize),
    pub(crate) announced_full_screen: Vec<(String, bool)>,
    pub(crate) announced_displays: Vec<String>,
    pub(crate) panel_anchors: Vec<crate::shell::PanelAnchor>,
    scanout_node: Option<DrmNode>,
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    dmabuf_state: DmabufState,
    _dmabuf_global: DmabufGlobal,
    _output_manager_state: OutputManagerState,
    _presentation_state: PresentationState,
    layer_shell_state: WlrLayerShellState,
    activation_state: XdgActivationState,
    _viewporter_state: ViewporterState,
    _fractional_scale_state: FractionalScaleManagerState,
    _cursor_shape_state: CursorShapeManagerState,
    _pointer_gestures_state: PointerGesturesState,
    _decoration_state: XdgDecorationState,
    _dialog_state: XdgDialogState,
    _text_input_state: TextInputManagerState,
    _input_method_state: InputMethodManagerState,
    _virtual_keyboard_state: VirtualKeyboardManagerState,
    pending_activation: Option<WlSurface>,
    pub(crate) windowless_app: Option<(Client, String)>,
    pub(crate) key_behind_layer: Option<WlSurface>,
    pub(crate) shell_resources: Vec<UnboundShellV1>,
    pub(crate) capture_resources:
        Vec<unbound_protocols::server::unbound_capture_v1::UnboundCaptureV1>,
    pub(crate) pending_insert: Option<crate::shell::PendingInsert>,
    pub(crate) app_resources: Vec<UnboundAppV1>,
    pub(crate) app_control_resources: Vec<UnboundAppControlV1>,
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
    pub(crate) session_end_requested:
        Option<unbound_protocols::server::unbound_shell_v1::SessionEnd>,
    pub(crate) quitting: Vec<(UnboundShellV1, Option<(String, u32)>)>,
    pub(crate) refused_tokens: Vec<XdgActivationToken>,
    pub(crate) drag: Option<crate::drag::Drag>,
    pub(crate) slide_back: Option<crate::scene::SlideBack>,
    pub(crate) watchdog: crate::watchdog::Watchdog,
    pub(crate) desktop: crate::gesture::ShowDesktop,
    pub(crate) overview: crate::overview::Overview,
    pub(crate) switcher: crate::switcher::Switcher,
    pub(crate) assist: crate::assist::Assist,
    pub(crate) recall: crate::recall::Recall,
    pub(crate) gesture_route: Option<crate::gesture::Route>,
    pub(crate) active_app: String,
    pub(crate) active_output: String,
    pub(crate) idle: crate::idle::Idle,
    _idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState,
    pub(crate) lock_states: Option<std::sync::mpsc::Sender<bool>>,
    pub(crate) lock: Option<Lock>,
    pub(crate) session_lock_state: SessionLockManagerState,
    pub(crate) popup_grab: Option<PopupGrab<State>>,
    pub(crate) held_window: Option<crate::grab::HeldWindow>,
    pub(crate) swallowed_button: Option<u32>,
    pub(crate) minimize_sequence: u64,
    pub(crate) closing: Vec<Closing>,
    pub(crate) icon_rects: Vec<IconRect>,
    pub(crate) snap: Option<SnapDrag>,
    pub(crate) preview: Option<Preview>,
    pub(crate) reduce_motion: bool,
    pub(crate) zoom: crate::zoom::Zoom,
    pub(crate) assistive: crate::assistive::Assistive,
    pub(crate) keys: crate::access_keys::AccessKeys,
    pub(crate) keyboard_monitor: crate::keyboard_monitor::KeyboardMonitor,
    pub(crate) needs_redraw: bool,
    pub(crate) reveal: crate::reveal::Reveal,
    pub(crate) pointer_location: Point<f64, Logical>,
    pub(crate) cursor_shown: bool,
    pub(crate) cursor_status: CursorImageStatus,
    pub(crate) cursor_theme: CursorTheme,
    cursor_arrow: Cursor,
    named_cursors: HashMap<CursorIcon, Option<Cursor>>,
    cursor_scale: f64,
    pub(crate) held_shortcut_keys: Vec<u32>,
    pub(crate) typed_at: Option<Instant>,
    pub(crate) input_devices: usize,
    pub(crate) input_sources: crate::keymap::InputSources,
    pub(crate) pointing: crate::pointing::Pointing,
    pub(crate) pending_captures: Vec<crate::capture::PendingCapture>,
    pub(crate) decorations: crate::frame::Decorations,
    pub(crate) alerts: crate::alert::Alerts,
}

pub struct Headless {
    display: Display<State>,
    pub(crate) fences: smithay::reexports::calloop::EventLoop<'static, State>,
    pub(crate) state: State,
    pub(crate) session_clients: Vec<SessionClient>,
    pub(crate) lock_screen: LockScreen,
    pub(crate) force_quit: Summoned,
    pub(crate) session_dialog: Summoned,
    pub(crate) socket_name: Option<OsString>,
}

impl Headless {
    pub fn new(output_size: Size<i32, Physical>) -> io::Result<Self> {
        Self::with_output(OutputConfig::virtual_display("HEADLESS-1", output_size))
    }

    pub fn with_output(config: OutputConfig) -> io::Result<Self> {
        Self::on_device(config, None)
    }

    pub fn on_device(config: OutputConfig, scanout: Option<ScanoutDevice>) -> io::Result<Self> {
        let mode = config
            .preferred_mode()
            .ok_or_else(|| io::Error::other(format!("{} has no mode", config.name)))?;
        let settings = OutputSettings {
            mode,
            scale: 1.0,
            transform: Transform::Normal,
            position: Point::default(),
        };
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
        let dmabuf_global = match &scanout {
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
        let center = settings.logical_size().to_f64().downscale(2.0);
        let screen = Screen::new(&handle, config, settings);
        handle.create_global::<State, UnboundShellV1, ()>(18, ());
        handle.create_global::<State, UnboundAppV1, ()>(6, ());
        handle.create_global::<State, UnboundAppControlV1, ()>(1, ());
        handle.create_global::<State, ExtBackgroundEffectManagerV1, ()>(1, ());
        handle.create_global::<State, smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::ZwlrOutputManagerV1, ()>(crate::management::MANAGER_VERSION, ());
        State::create_capture_globals(&handle);
        State::create_capture_shell_global(&handle);
        let state = State {
            display_handle: handle.clone(),
            syncobj_state: None,
            blockers: fences.handle(),
            screens: vec![screen],
            departed: Vec::new(),
            display_config: unbound_config::Config::default(),
            night_light: crate::night::NightLight::default(),
            output_managers: Vec::new(),
            output_serial: 0,
            announced_workspaces: (1, 0),
            announced_full_screen: Vec::new(),
            announced_displays: Vec::new(),
            panel_anchors: Vec::new(),
            scanout_node: scanout.as_ref().map(|scanout| scanout.node),
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
            _output_manager_state: OutputManagerState::new(),
            _presentation_state: PresentationState::new::<State>(&handle, MONOTONIC),
            layer_shell_state: WlrLayerShellState::new::<State>(&handle),
            activation_state: XdgActivationState::new::<State>(&handle),
            _viewporter_state: ViewporterState::new::<State>(&handle),
            _fractional_scale_state: FractionalScaleManagerState::new::<State>(&handle),
            _decoration_state: XdgDecorationState::new::<State>(&handle),
            _dialog_state: XdgDialogState::new::<State>(&handle),
            _cursor_shape_state: CursorShapeManagerState::new::<State>(&handle),
            _pointer_gestures_state: PointerGesturesState::new::<State>(&handle),
            _text_input_state: TextInputManagerState::new::<State>(&handle),
            _input_method_state: InputMethodManagerState::new::<State, _>(&handle, |_| true),
            _virtual_keyboard_state: VirtualKeyboardManagerState::new::<State, _>(&handle, |_| {
                true
            }),
            pending_activation: None,
            windowless_app: None,
            key_behind_layer: None,
            shell_resources: Vec::new(),
            capture_resources: Vec::new(),
            pending_insert: None,
            app_resources: Vec::new(),
            app_control_resources: Vec::new(),
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
            session_end_requested: None,
            quitting: Vec::new(),
            refused_tokens: Vec::new(),
            drag: None,
            slide_back: None,
            watchdog: crate::watchdog::Watchdog::default(),
            desktop: crate::gesture::ShowDesktop::default(),
            overview: crate::overview::Overview::default(),
            switcher: crate::switcher::Switcher::default(),
            assist: crate::assist::Assist::default(),
            recall: crate::recall::Recall::default(),
            gesture_route: None,
            active_app: String::new(),
            active_output: String::new(),
            idle: crate::idle::Idle::default(),
            _idle_inhibit_state: smithay::wayland::idle_inhibit::IdleInhibitManagerState::new::<
                State,
            >(&handle),
            lock_states: None,
            lock: None,
            session_lock_state: SessionLockManagerState::new::<State, _>(&handle, shell::is_shell),
            popup_grab: None,
            held_window: None,
            swallowed_button: None,
            minimize_sequence: 0,
            closing: Vec::new(),
            icon_rects: Vec::new(),
            snap: None,
            preview: None,
            reduce_motion: false,
            zoom: crate::zoom::Zoom::default(),
            assistive: crate::assistive::Assistive::default(),
            keys: crate::access_keys::AccessKeys::default(),
            keyboard_monitor: crate::keyboard_monitor::KeyboardMonitor::default(),
            needs_redraw: false,
            reveal: crate::reveal::Reveal::default(),
            pointer_location: (center.w, center.h).into(),
            cursor_shown: false,
            cursor_status: CursorImageStatus::default_named(),
            cursor_theme: CursorTheme::builtin(),
            cursor_arrow: Cursor {
                buffer: cursor::arrow(),
                hotspot: Point::default(),
            },
            named_cursors: HashMap::new(),
            cursor_scale: 1.0,
            held_shortcut_keys: Vec::new(),
            typed_at: None,
            input_devices: 0,
            input_sources: crate::keymap::InputSources::default(),
            pointing: crate::pointing::Pointing::default(),
            pending_captures: Vec::new(),
            decorations: crate::frame::Decorations::default(),
            alerts: crate::alert::Alerts::default(),
        };
        Ok(Self {
            display,
            fences,
            state,
            session_clients: Vec::new(),
            lock_screen: LockScreen::new(),
            force_quit: Summoned::new(crate::session::MONITOR),
            session_dialog: Summoned::new(crate::session::SHELL),
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
        self.state.deliver_pending_insert();
        self.state.activate_next_app_if_the_windowless_one_quit();
        self.state.announce_apps_running();
        self.state.popups.cleanup();
        self.state.end_orphaned_drag();
        self.state.keep_families_together();
        self.state.refocus_pointer();
        self.display.flush_clients()
    }

    pub fn poll_fd(&mut self) -> io::Result<OwnedFd> {
        self.display.backend().poll_fd().try_clone_to_owned()
    }

    pub fn take_redraw(&mut self) -> bool {
        self.take_screen_redraw(0)
    }

    pub fn take_redraw_on(&mut self, output: &str) -> bool {
        self.state
            .screen_named(output)
            .is_some_and(|screen| self.take_screen_redraw(screen))
    }

    fn take_screen_redraw(&mut self, screen: usize) -> bool {
        if std::mem::take(&mut self.state.needs_redraw) {
            for screen in &mut self.state.screens {
                screen.redraw = true;
            }
        }
        std::mem::take(&mut self.state.screens[screen].redraw)
    }

    pub fn latch(&mut self) {
        self.latch_screen(0);
    }

    pub fn present(&mut self, vblank: Vblank) -> io::Result<()> {
        self.present_screen(0, vblank)
    }

    pub fn latch_on(&mut self, output: &str) {
        if let Some(screen) = self.state.screen_named(output) {
            self.latch_screen(screen);
        }
    }

    pub fn present_on(&mut self, output: &str, vblank: Vblank) -> io::Result<()> {
        match self.state.screen_named(output) {
            Some(screen) => self.present_screen(screen, vblank),
            None => Ok(()),
        }
    }

    fn latch_screen(&mut self, screen: usize) {
        let state = &mut self.state;
        state.latch_lock(screen);
        let unmapped = state
            .windows
            .iter()
            .filter(|window| state.screen_of(window) == screen)
            .filter(|window| {
                window.size.is_none() || (state.overview.is_shown() && window.visible())
            })
            .map(|window| window.surface().clone());
        let roots = state
            .stack(screen)
            .into_iter()
            .map(|(surface, _)| surface)
            .chain(unmapped)
            .collect::<Vec<_>>();
        let latched = &mut state.screens[screen];
        for root in roots {
            with_surface_tree_downward(
                &root,
                (),
                |_, _, _| TraversalAction::DoChildren(()),
                |_, states, _| {
                    latched.latched_feedback.append(
                        &mut states
                            .cached_state
                            .get::<PresentationFeedbackCachedState>()
                            .current()
                            .callbacks,
                    );
                    latched.latched_frame_callbacks.append(
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

    fn present_screen(&mut self, screen: usize, vblank: Vblank) -> io::Result<()> {
        let presented = &mut self.state.screens[screen];
        let refresh = presented.refresh();
        for feedback in presented.latched_feedback.drain(..) {
            feedback.presented(
                &presented.output,
                vblank.time,
                Refresh::fixed(refresh),
                vblank.sequence,
                Kind::Vsync,
            );
        }
        let millis = (vblank.time.as_millis() % (1 << 32)) as u32;
        for callback in presented.latched_frame_callbacks.drain(..) {
            callback.done(millis);
        }
        self.state.confirm_lock(screen);
        self.display.flush_clients()
    }

    pub fn output_size(&self) -> Size<i32, Physical> {
        self.state.main_screen().mode_size()
    }

    pub fn output_scale(&self) -> f64 {
        self.state.main_screen().scale()
    }

    pub fn logical_output_size(&self) -> Size<i32, Logical> {
        self.state.main_screen().rect().size
    }

    pub fn set_output_scale(&mut self, scale: f64) {
        let name = self.state.main_screen().name();
        let settings = OutputSettings {
            scale,
            ..self.state.main_screen().settings
        };
        self.configure_output(&name, settings);
    }

    pub fn windows(&self) -> &[Window] {
        &self.state.windows
    }

    pub fn render_elements(&mut self, renderer: &mut VulkanRenderer) -> Vec<ScreenElement> {
        self.render_screen(renderer, 0)
    }

    pub fn render_elements_on(
        &mut self,
        renderer: &mut VulkanRenderer,
        output: &str,
    ) -> Vec<ScreenElement> {
        match self.state.screen_named(output) {
            Some(screen) => self.render_screen(renderer, screen),
            None => Vec::new(),
        }
    }

    fn render_screen(
        &mut self,
        renderer: &mut VulkanRenderer,
        screen: usize,
    ) -> Vec<ScreenElement> {
        self.state.paint_title_bars(renderer);
        self.state.prepare_switcher(screen);
        let (elements, drawn, icon) = self.compose(renderer, screen, true);
        if self.state.pointer_screen() == screen {
            self.state.remember_drag_icon(icon);
        }
        for index in 0..self.state.windows.len() {
            if self.state.screen_of(&self.state.windows[index]) == screen {
                self.state.windows[index].drawn = None;
            }
        }
        for (surface, drawn) in drawn {
            if let Some(index) = self.state.window_index(&surface)
                && self.state.screen_of(&self.state.windows[index]) == screen
            {
                self.state.windows[index].drawn = drawn;
            }
        }
        elements
    }

    pub(crate) fn scene_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        with_cursor: bool,
    ) -> Vec<ScreenElement> {
        self.compose(renderer, screen, with_cursor).0
    }

    fn compose(&self, renderer: &mut VulkanRenderer, screen: usize, with_cursor: bool) -> Composed {
        if let Some(source) = self.state.screens[screen]
            .mirror
            .as_deref()
            .and_then(|source| self.state.screen_named(source))
        {
            return self.compose_mirror(renderer, screen, source, with_cursor);
        }
        let scale = self.state.render_scale(screen);
        let (elements, drawn, icon) = self.compose_at(renderer, screen, scale, with_cursor);
        let origin = self.state.view_origin_in_pixels(screen, scale);
        (relocated(elements, Point::default() - origin), drawn, icon)
    }

    fn compose_mirror(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        source: usize,
        with_cursor: bool,
    ) -> Composed {
        let mirror = &self.state.screens[screen];
        let scale = mirror.scale();
        let frame = mirror.settings.transform.transform_size(mirror.mode_size());
        let shown = self
            .state
            .screen_rect(source)
            .size
            .to_f64()
            .to_physical(scale)
            .to_i32_round::<i32>();
        let inset =
            Point::<i32, Physical>::from(((frame.w - shown.w) / 2, (frame.h - shown.h) / 2));
        let (elements, _, _) = self.compose_at(renderer, source, scale, with_cursor);
        let origin = self.state.screens[source].origin_in_pixels(scale);
        (relocated(elements, inset - origin), Vec::new(), None)
    }

    fn compose_at(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
        with_cursor: bool,
    ) -> Composed<OutputElement> {
        let mut drawn = Vec::new();
        let pointer_here = self.state.pointer_screen() == screen;
        let (dragged, icon) = match with_cursor {
            true => self.state.drag_icon_elements(renderer, scale),
            false => (Vec::new(), None),
        };
        let mut elements: Vec<OutputElement> = match with_cursor && pointer_here {
            true => self.cursor_elements(renderer, scale),
            false => Vec::new(),
        };
        elements.extend(dragged);
        elements.extend(self.state.alert_elements(renderer, screen, scale));
        elements.extend(self.state.flash_elements(screen, scale));
        elements.extend(self.state.source_hud_element(renderer, screen, scale));
        elements.extend(self.state.switcher_elements(renderer, screen, scale));
        elements.extend(self.state.assist_elements(renderer, screen, scale));
        let key = self.state.key_window();
        let shell_opacity = self.state.shell_opacity();
        let wallpapers: Vec<WlSurface> = match shell_opacity < 1.0 {
            true => self
                .state
                .placed_layers(screen, Layer::Background)
                .into_iter()
                .map(|(surface, _)| surface)
                .collect(),
            false => Vec::new(),
        };
        for placed in self.state.scene(true, screen) {
            match placed {
                Placed::Surface(surface, location) => {
                    let alpha = match wallpapers.contains(&surface) {
                        true => 1.0,
                        false => shell_opacity,
                    };
                    elements.extend(render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        location.to_f64().to_physical(scale).to_i32_round::<i32>(),
                        scale,
                        alpha,
                        ElementKind::Unspecified,
                    ));
                    elements.extend(surface_blurs(
                        &surface,
                        location,
                        scale,
                        self.state
                            .backdrop()
                            .or_else(|| scrim(renderer, self.state.decorations.scheme)),
                    ));
                }
                Placed::Window(window) => {
                    let shading = Shading {
                        active: key.as_ref() == Some(window.surface()),
                        scheme: self.state.decorations.scheme,
                    };
                    let hung = window
                        .surface()
                        .client()
                        .is_some_and(|owner| self.state.watchdog.is_unresponsive(&owner.id()));
                    let presented = self.state.presented(window);
                    let (shown, look) = window_elements(
                        window,
                        renderer,
                        scale,
                        shading,
                        hung,
                        presented,
                        self.state.backdrop(),
                    );
                    elements.extend(shown);
                    elements.extend(self.state.overview_ring(window, scale));
                    if let (Some(ghost), Some(look)) = (self.state.overview_ghost(window), &look) {
                        elements.extend(look.ghost(ghost, scale));
                    }
                    drawn.push((window.surface().clone(), look));
                    if let Some(preview) = &self.state.preview
                        && &preview.owner == window.surface()
                    {
                        elements.extend(preview_elements(
                            preview,
                            scale,
                            self.state.decorations.scheme,
                        ));
                    }
                }
                Placed::Closing(closing) => elements.extend(closing.render_elements(scale)),
                Placed::Overview(screen) => {
                    elements.extend(self.state.overview_elements(renderer, screen, scale));
                }
                Placed::Backdrop(screen) => {
                    elements.extend(self.state.overview_backdrop(
                        screen,
                        scale,
                        renderer.reduced_effects(),
                    ));
                }
                Placed::Scrim(owner, screen) => {
                    if let Some(scrim) = crate::effect::scrim(&owner) {
                        elements.extend(self.state.dim_screen(
                            screen,
                            scale,
                            renderer.reduced_effects(),
                            scrim.ids,
                            scrim.opacity,
                        ));
                    }
                }
            }
        }
        tie_blurs_to_what_lies_below(&mut elements, scale);
        (elements, drawn, icon)
    }

    pub fn popup_count(&self) -> usize {
        (0..self.state.screens.len())
            .flat_map(|screen| self.state.stack(screen))
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
        self.state.usable_area(0)
    }

    pub(crate) fn cursor_elements(
        &self,
        renderer: &mut VulkanRenderer,
        scale: f64,
    ) -> Vec<OutputElement> {
        let state = &self.state;
        if !state.cursor_shown {
            return Vec::new();
        }
        let at: Point<i32, Physical> = state.pointer_location.to_physical(scale).to_i32_round();
        let busy = CursorImageStatus::Named(CursorIcon::Wait);
        let dragging = state.drag_cursor().map(CursorImageStatus::Named);
        let over_hung_app = state
            .seat
            .get_pointer()
            .and_then(|pointer| pointer.current_focus())
            .and_then(|surface| surface.client())
            .is_some_and(|owner| state.watchdog.is_unresponsive(&owner.id()));
        let status = match &dragging {
            Some(dragging) => dragging,
            None if over_hung_app => &busy,
            None => &state.cursor_status,
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
                .map(OutputElement::Memory)
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
    pub(crate) departed: AtomicBool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        self.departed.store(true, Ordering::Relaxed);
    }
}

const MONOTONIC: u32 = 1;
const ACTIVATION_TOKEN_LIFETIME: Duration = Duration::from_secs(60);

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
        let scale = self.screens[self.pointer_screen()].scale();
        if scale != self.cursor_scale {
            self.cursor_scale = scale;
            self.named_cursors.clear();
        }
        let theme = &self.cursor_theme;
        self.named_cursors
            .entry(icon)
            .or_insert_with(|| theme.load(icon, scale));
    }

    pub(crate) fn reload_cursor(&mut self) {
        if let CursorImageStatus::Named(icon) = self.cursor_status {
            self.load_named_cursor(icon);
        }
        if let Some(icon) = self.drag_cursor() {
            self.load_named_cursor(icon);
        }
    }

    pub(crate) fn use_cursor_theme(&mut self, theme: CursorTheme) {
        self.cursor_theme = theme;
        self.named_cursors.clear();
        self.reload_cursor();
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
        let surface = surface.map(|surface| self.modal_blocker(&surface).unwrap_or(surface));
        if self.popup_grab.is_some() && surface != self.menu_root() {
            self.close_menus();
        }
        let onto_layer = surface
            .as_ref()
            .is_some_and(|surface| self.toplevel_of(surface).is_none());
        self.key_behind_layer = self.key_window().filter(|_| onto_layer);
        if !onto_layer {
            self.windowless_app = None;
        }
        self.typed_at = None;
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
            let holder = exclusive.surface.clone();
            if let Some(index) = surface
                .as_ref()
                .and_then(|surface| self.window_index(surface))
            {
                self.raise(index);
            }
            if self.popup_grab.is_none()
                && let Some(keyboard) = self.seat.get_keyboard()
                && keyboard.current_focus().as_ref() != Some(&holder)
            {
                keyboard.set_focus(self, Some(holder), SERIAL_COUNTER.next_serial());
            }
            self.sync_activated();
            return;
        }
        if let Some(index) = surface
            .as_ref()
            .and_then(|surface| self.window_index(surface))
        {
            self.raise(index);
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

    pub(crate) fn frontmost_on(&self, screen: usize) -> Option<WlSurface> {
        self.windows
            .iter()
            .rev()
            .filter(|window| self.present(window) && self.screen_of(window) == screen)
            .map(|window| window.toplevel.wl_surface().clone())
            .next()
    }

    pub(crate) fn refocus_after_losing(&mut self, lost: &WlSurface) {
        let Some(focused) = self.focused() else {
            return;
        };
        if &focused != lost && focused.is_alive() && self.root_of(&focused).as_ref() != Some(lost) {
            return;
        }
        let owner = lost.client();
        let next = match &owner {
            Some(owner) => self.frontmost(Some(owner)),
            None => self.frontmost(None),
        };
        let windowless = next.is_none();
        let app_id = shell::toplevel_names(lost).1;
        self.focus(next);
        if windowless {
            self.windowless_app = owner.map(|owner| (owner, app_id));
            let key = self.key_window();
            self.announce_active_app(key.as_ref());
        }
    }

    fn activate_next_app_if_the_windowless_one_quit(&mut self) {
        let quit = self.windowless_app.as_ref().is_some_and(|(app, _)| {
            app.get_data::<ClientState>()
                .is_some_and(|state| state.departed.load(Ordering::Relaxed))
        });
        if quit {
            self.windowless_app = None;
            if self.focused().is_none() {
                let next = self.frontmost(None);
                self.focus(next);
            }
        }
    }

    fn takes_focus_when_mapped(&self, surface: &WlSurface) -> bool {
        let Some(focused) = self.focused() else {
            return true;
        };
        let owner = surface.client();
        if focused.client() == owner {
            return true;
        }
        if self.would_interrupt_typing(surface, &focused) {
            return false;
        }
        !self.windows.iter().any(|window| {
            window.size.is_some()
                && window.toplevel.wl_surface() != surface
                && window.toplevel.wl_surface().client() == owner
        })
    }

    fn place_behind_frontmost(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        if self.stack_above_parent(index) {
            return;
        }
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
        area.location.x + ((area.size.w - window.w) / 2).max(0),
        area.location.y + ((area.size.h - window.h) / 2).max(0),
    )
        .into()
}

impl State {
    pub(crate) fn placed_layers(
        &self,
        screen: usize,
        layer: Layer,
    ) -> Vec<(WlSurface, Point<i32, Logical>)> {
        let origin = self.screens[screen].settings.position;
        let map = layer_map_for_output(&self.screens[screen].output);
        map.layers_on(layer)
            .rev()
            .filter_map(|mapped| {
                let geometry = map.layer_geometry(mapped)?;
                Some((mapped.wl_surface().clone(), origin + geometry.loc))
            })
            .collect()
    }

    pub(crate) fn layer_screen(&self, surface: &WlSurface) -> Option<usize> {
        self.screens.iter().position(|screen| {
            layer_map_for_output(&screen.output)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some()
        })
    }

    pub(crate) fn layer_origin(&self, surface: &WlSurface) -> Option<Point<i32, Logical>> {
        let screen = &self.screens[self.layer_screen(surface)?];
        let map = layer_map_for_output(&screen.output);
        let layer = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)?;
        let geometry = map.layer_geometry(layer)?;
        Some(screen.settings.position + geometry.loc)
    }

    pub(crate) fn stack(&self, screen: usize) -> Vec<(WlSurface, Point<i32, Logical>)> {
        self.scene(false, screen)
            .into_iter()
            .filter_map(|placed| match placed {
                Placed::Surface(surface, origin) => Some((surface, origin)),
                Placed::Window(window) => Some((window.surface().clone(), window.surface_origin())),
                Placed::Closing(_)
                | Placed::Overview(_)
                | Placed::Backdrop(_)
                | Placed::Scrim(..) => None,
            })
            .collect()
    }

    pub(crate) fn scene(&self, rendered: bool, screen: usize) -> Vec<Placed<'_>> {
        if self.is_locked() {
            let locked = self.locked_stack(screen);
            let wallpaper = if rendered && locked.is_empty() && self.lock_shown() {
                self.placed_layers(screen, Layer::Background)
            } else {
                Vec::new()
            };
            return locked
                .into_iter()
                .chain(wallpaper)
                .map(|(surface, origin)| Placed::Surface(surface, origin))
                .collect();
        }

        let popups = |surface: &WlSurface, geometry_origin| {
            self.popup_surfaces(surface, geometry_origin)
                .into_iter()
                .map(|(surface, origin)| Placed::Surface(surface, origin))
        };
        let layers = |layer: Layer| {
            self.placed_layers(screen, layer)
                .into_iter()
                .flat_map(|(surface, at)| {
                    let scrim = crate::effect::scrim(&surface)
                        .filter(|_| is_mapped(&surface))
                        .map(|_| Placed::Scrim(surface.clone(), screen));
                    popups(&surface, at)
                        .chain([Placed::Surface(surface, at)])
                        .chain(scrim)
                })
                .collect::<Vec<_>>()
        };
        let visible = self
            .windows
            .iter()
            .rev()
            .filter(|window| {
                self.screen_of(window) == screen || (rendered && self.dragged_onto(window, screen))
            })
            .filter(|window| {
                if rendered {
                    window.rendered() && self.on_view(window)
                } else {
                    self.present(window)
                        && !self.desktop.hides_windows()
                        && !self.overview.is_shown()
                }
            })
            .collect::<Vec<_>>();
        let (front, rest): (Vec<&Window>, Vec<&Window>) = visible.into_iter().partition(|window| {
            self.on_full_screen_space(window) || self.dragged_in_overview(window)
        });
        let (leaving, rest): (Vec<&Window>, Vec<&Window>) =
            rest.into_iter().partition(|window| window.is_minimized());
        let (on_top, rest): (Vec<&Window>, Vec<&Window>) =
            rest.into_iter().partition(|window| window.keep_on_top);
        let name = self.screens[screen].name();
        [
            layers(Layer::Overlay),
            self.window_entries(&leaving),
            self.window_entries(&front),
            layers(Layer::Top),
            if rendered && self.overview.is_shown() {
                vec![Placed::Overview(screen)]
            } else {
                Vec::new()
            },
            self.closing
                .iter()
                .filter(|closing| rendered && closing.output == name)
                .map(Placed::Closing)
                .collect(),
            self.window_entries(&on_top),
            self.window_entries(&rest),
            layers(Layer::Bottom),
            if rendered && self.overview.is_shown() {
                vec![Placed::Backdrop(screen)]
            } else {
                Vec::new()
            },
            layers(Layer::Background),
        ]
        .concat()
    }

    fn window_entries<'a>(&'a self, windows: &[&'a Window]) -> Vec<Placed<'a>> {
        windows
            .iter()
            .flat_map(|window| {
                self.popup_surfaces(window.surface(), window.content_location())
                    .into_iter()
                    .map(|(surface, origin)| Placed::Surface(surface, origin))
                    .chain([Placed::Window(window)])
            })
            .collect()
    }

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
                self.sync_surface_outputs();
            }
            (false, true) => {
                if self
                    .focused()
                    .is_some_and(|focused| self.would_interrupt_typing(surface, &focused))
                {
                    self.request_attention(shell::toplevel_names(surface).1);
                }
                self.place_behind_frontmost(surface);
                self.sync_surface_outputs();
            }
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

render_elements! {
    pub OutputElement<=VulkanRenderer>;
    Surface=WaylandSurfaceRenderElement<VulkanRenderer>,
    Window=WindowElement<WaylandSurfaceRenderElement<VulkanRenderer>>,
    Shadow=ShadowElement,
    Closing=WindowElement<PieceElement>,
    Fill=FillElement,
    Blur=BlurElement,
    Memory=MemoryRenderBufferRenderElement<VulkanRenderer>,
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
        self.activation_state
            .retain_tokens(|_, data| data.timestamp.elapsed() < ACTIVATION_TOKEN_LIFETIME);
        let known = &self.activation_state;
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
        self.activation_state.remove_token(&token);
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

pub(crate) fn is_mapped(surface: &WlSurface) -> bool {
    with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false)
}
