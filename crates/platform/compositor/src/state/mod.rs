use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use smithay::backend::allocator::Buffer;
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::WaylandSurfaceRenderElement;
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::desktop::{PopupGrab, PopupManager};
use smithay::input::keyboard::XkbConfig;
use smithay::input::pointer::{CursorIcon, CursorImageStatus};
use smithay::render_elements;

use smithay::input::Seat;
use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback::Kind;
use smithay::reexports::wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle};
use smithay::utils::{Logical, Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::compositor::{
    CompositorClientState, SurfaceAttributes,
    TraversalAction, with_surface_tree_downward,
};
use smithay::wayland::dmabuf::{
    DmabufFeedback, get_dmabuf,
};
use smithay::wayland::fractional_scale::with_fractional_scale;
use smithay::wayland::presentation::{
    PresentationFeedbackCachedState, Refresh,
};
use smithay::wayland::shell::wlr_layer::Layer;
use smithay::wayland::shm::with_buffer_contents;
use smithay::wayland::xdg_activation::XdgActivationToken;
use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use bande_protocols::server::bande_app_control_v1::BandeAppControlV1;
use bande_protocols::server::bande_app_v1::BandeAppV1;
use bande_protocols::server::bande_shell_v1::BandeShellV1;
use bande_render::VulkanRenderer;

use crate::clock::Vblank;
use crate::cursor::{self, Cursor, CursorTheme};
use crate::lock::Lock;
use crate::motion::IconRect;
use crate::scene::{BlurElement, Closing, FillElement, PieceElement, ShadowElement, WindowElement};
use crate::screen::{OutputConfig, OutputSettings, Screen};
use crate::session::{LockScreen, SessionClient, SessionRole, Summoned};
use crate::tile::{Preview, SnapDrag};
use crate::window::Window;

mod compose;
mod focus;
mod globals;
mod handlers;

pub(crate) use compose::Placed;
use globals::Globals;

pub const REFRESH: Duration = Duration::from_nanos(1_000_000_000 / 120);

pub struct ScanoutDevice {
    pub node: DrmNode,
    pub feedback: DmabufFeedback,
}

pub(crate) const MENU_BAR_NAMESPACE: &str = "menubar";
pub(crate) const MENU_SEARCH_NAMESPACE: &str = "menu-search";

pub(crate) struct ExclusiveLayer {
    pub(crate) surface: WlSurface,
    pub(crate) previous: Option<WlSurface>,
}

pub struct State {
    pub(crate) display_handle: DisplayHandle,
    pub(crate) globals: Globals,
    pub(crate) blockers: smithay::reexports::calloop::LoopHandle<'static, State>,
    pub(crate) screens: Vec<Screen>,
    pub(crate) departed: Vec<crate::displays::Departed>,
    pub(crate) display_config: bande_config::Config,
    pub(crate) night_light: crate::night::NightLight,
    pub(crate) output_managers: Vec<crate::management::Manager>,
    pub(crate) output_serial: u32,
    pub(crate) announced_workspaces: (usize, usize),
    pub(crate) announced_full_screen: Vec<(String, bool)>,
    pub(crate) announced_displays: Vec<String>,
    pub(crate) panel_anchors: Vec<crate::shell::PanelAnchor>,
    scanout_node: Option<DrmNode>,
    pending_activation: Option<WlSurface>,
    pub(crate) windowless_app: Option<(Client, String)>,
    pub(crate) key_behind_layer: Option<WlSurface>,
    pub(crate) shell_resources: Vec<BandeShellV1>,
    pub(crate) capture_resources: Vec<bande_protocols::server::bande_capture_v1::BandeCaptureV1>,
    pub(crate) pending_insert: Option<crate::shell::PendingInsert>,
    pub(crate) app_resources: Vec<BandeAppV1>,
    pub(crate) kept_apps: Vec<(BandeAppV1, String)>,
    pub(crate) announced_windowless: Vec<(String, i32)>,
    pub(crate) app_control_resources: Vec<BandeAppControlV1>,
    pub(crate) exclusive_layer: Option<ExclusiveLayer>,
    exclusive_candidates: Vec<(WlSurface, Layer)>,
    pub(crate) seat: Seat<State>,
    pub(crate) windows: Vec<Window>,
    pub(crate) popups: PopupManager,
    pub(crate) lock_screen_requested: bool,
    pub(crate) force_quit: crate::force_quit::ForceQuit,
    pub(crate) session_end_requested: Option<bande_protocols::server::bande_shell_v1::SessionEnd>,
    pub(crate) quitting: Vec<(BandeShellV1, Option<(String, u32)>)>,
    pub(crate) refused_tokens: Vec<XdgActivationToken>,
    pub(crate) drag: Option<crate::drag::Drag>,
    pub(crate) slide_back: Option<crate::scene::SlideBack>,
    pub(crate) watchdog: crate::watchdog::Watchdog,
    pub(crate) desktop: crate::gesture::ShowDesktop,
    pub(crate) overview: crate::overview::Overview,
    pub(crate) switcher: crate::switcher::Switcher,
    pub(crate) assist: crate::assist::Assist,
    pub(crate) recall: crate::recall::Recall,
    pub(crate) reopen_file: Option<std::path::PathBuf>,
    pub(crate) running_apps: Option<crate::app::RunningApps>,
    pub(crate) apps_changed: bool,
    pub(crate) gesture_route: Option<crate::gesture::Route>,
    pub(crate) active_app: String,
    pub(crate) active_output: String,
    pub(crate) idle: crate::idle::Idle,
    pub(crate) output_power: crate::output_power::OutputPower,
    pub(crate) lock_states: Option<std::sync::mpsc::Sender<bool>>,
    pub(crate) lock: Option<Lock>,
    pub(crate) popup_grab: Option<PopupGrab<State>>,
    pub(crate) held_window: Option<crate::grab::HeldWindow>,
    pub(crate) swallowed_button: Option<u32>,
    pub(crate) virtual_terminal: Option<i32>,
    pub(crate) desktop_shell: Option<Client>,
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
    pub(crate) super_tap: bool,
    pub(crate) typed_at: Option<Instant>,
    pub(crate) input_devices: usize,
    pub(crate) input_sources: crate::keymap::InputSources,
    pub(crate) pointing: crate::pointing::Pointing,
    pub(crate) pending_captures: Vec<crate::capture::PendingCapture>,
    pub(crate) decorations: crate::frame::Decorations,
    pub(crate) window_menus: crate::window_menu::WindowMenus,
    pub(crate) alerts: crate::alert::Alerts,
    pub(crate) labels: std::cell::RefCell<crate::label::Labels>,
}

pub struct Compositor {
    display: Display<State>,
    pub(crate) fences: smithay::reexports::calloop::EventLoop<'static, State>,
    pub(crate) state: State,
    pub(crate) session_clients: Vec<SessionClient>,
    pub(crate) lock_screen: LockScreen,
    pub(crate) session_dialog: Summoned,
    pub(crate) socket_name: Option<OsString>,
}

impl Compositor {
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
        let mut globals = Globals::new(&handle, scanout.as_ref());
        let mut seat = globals.seat_state.new_wl_seat(&handle, "seat0");
        seat.add_keyboard(XkbConfig::default(), 600, 25)
            .map_err(io::Error::other)?;
        seat.add_pointer();
        crate::idle::create_global(&handle);
        crate::output_power::create_global(&handle);
        let center = settings.logical_size().to_f64().downscale(2.0);
        let screen = Screen::new(&handle, config, settings);
        handle.create_global::<State, BandeShellV1, ()>(1, ());
        handle.create_global::<State, BandeAppV1, ()>(3, ());
        handle.create_global::<State, BandeAppControlV1, ()>(1, ());
        handle.create_global::<State, ExtBackgroundEffectManagerV1, ()>(1, ());
        handle.create_global::<State, smithay::reexports::wayland_protocols_wlr::output_management::v1::server::zwlr_output_manager_v1::ZwlrOutputManagerV1, ()>(crate::management::MANAGER_VERSION, ());
        State::create_capture_globals(&handle);
        State::create_capture_shell_global(&handle);
        let mut state = State {
            display_handle: handle.clone(),
            globals,
            blockers: fences.handle(),
            screens: vec![screen],
            departed: Vec::new(),
            display_config: bande_config::Config::default(),
            night_light: crate::night::NightLight::default(),
            output_managers: Vec::new(),
            output_serial: 0,
            announced_workspaces: (1, 0),
            announced_full_screen: Vec::new(),
            announced_displays: Vec::new(),
            panel_anchors: Vec::new(),
            scanout_node: scanout.as_ref().map(|scanout| scanout.node),
            pending_activation: None,
            windowless_app: None,
            key_behind_layer: None,
            shell_resources: Vec::new(),
            capture_resources: Vec::new(),
            pending_insert: None,
            app_resources: Vec::new(),
            kept_apps: Vec::new(),
            announced_windowless: Vec::new(),
            app_control_resources: Vec::new(),
            exclusive_layer: None,
            exclusive_candidates: Vec::new(),
            seat,
            windows: Vec::new(),
            popups: PopupManager::default(),
            lock_screen_requested: false,
            force_quit: crate::force_quit::ForceQuit::default(),
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
            reopen_file: None,
            running_apps: None,
            apps_changed: false,
            gesture_route: None,
            active_app: String::new(),
            active_output: String::new(),
            idle: crate::idle::Idle::default(),
            output_power: crate::output_power::OutputPower::default(),
            lock_states: None,
            lock: None,
            popup_grab: None,
            held_window: None,
            swallowed_button: None,
            virtual_terminal: None,
            desktop_shell: None,
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
            super_tap: false,
            typed_at: None,
            input_devices: 0,
            input_sources: crate::keymap::InputSources::default(),
            pointing: crate::pointing::Pointing::default(),
            pending_captures: Vec::new(),
            decorations: crate::frame::Decorations::default(),
            window_menus: crate::window_menu::WindowMenus::default(),
            alerts: crate::alert::Alerts::default(),
            labels: std::cell::RefCell::default(),
        };
        let (catalog, arrivals) = smithay::reexports::calloop::channel::channel();
        fences
            .handle()
            .insert_source(arrivals, |event, _, state: &mut State| {
                if let smithay::reexports::calloop::channel::Event::Msg(loaded) = event {
                    state.receive_catalog(loaded);
                }
            })
            .map_err(|error| io::Error::other(error.error))?;
        state.switcher.load_with(catalog);
        let (actions, requests) = smithay::reexports::calloop::channel::channel();
        fences
            .handle()
            .insert_source(requests, |event, _, state: &mut State| {
                if let smithay::reexports::calloop::channel::Event::Msg(request) = event {
                    state.force_quit_action(request);
                }
            })
            .map_err(|error| io::Error::other(error.error))?;
        state.force_quit.take_actions(actions);
        Ok(Self {
            display,
            fences,
            state,
            session_clients: Vec::new(),
            lock_screen: LockScreen::new(),
            session_dialog: Summoned::new(crate::session::SHELL),
            socket_name: None,
        })
    }

    pub fn insert_client(&mut self, stream: UnixStream) -> io::Result<Client> {
        let state = ClientState {
            process: crate::force_quit::Process::of_peer(&stream),
            ..ClientState::default()
        };
        self.display.handle().insert_client(stream, Arc::new(state))
    }

    pub(crate) fn display_handle(&self) -> DisplayHandle {
        self.display.handle()
    }

    pub fn dispatch(&mut self) -> io::Result<()> {
        self.clear_signalled_fences()?;
        self.display.dispatch_clients(&mut self.state)?;
        self.state.deliver_pending_insert();
        self.state.activate_next_app_if_the_windowless_one_quit();
        self.state.announce_windowless_apps();
        self.state.announce_apps_running();
        self.state.note_running_apps();
        self.state.popups.cleanup();
        self.state.end_orphaned_drag();
        self.state.keep_families_together();
        self.state.refocus_pointer();
        self.display.flush_clients()
    }

    pub fn poll_fd(&mut self) -> io::Result<OwnedFd> {
        self.display.backend().poll_fd().try_clone_to_owned()
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

    pub fn use_cursor_theme(&mut self, theme: CursorTheme) {
        self.state.use_cursor_theme(theme);
    }

    pub fn cursor_theme(&self) -> &CursorTheme {
        &self.state.cursor_theme
    }

    pub fn usable_area(&self) -> Rectangle<i32, Logical> {
        self.state.usable_area(0)
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
    pub(crate) process: Option<crate::force_quit::Process>,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        self.departed.store(true, Ordering::Relaxed);
    }
}

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
}

pub(crate) fn centered(
    area: Rectangle<i32, Logical>,
    window: Size<i32, Logical>,
) -> Point<i32, Logical> {
    (
        area.loc.x + ((area.size.w - window.w) / 2).max(0),
        area.loc.y + ((area.size.h - window.h) / 2).max(0),
    )
        .into()
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

pub(crate) fn is_mapped(surface: &WlSurface) -> bool {
    with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false)
}
