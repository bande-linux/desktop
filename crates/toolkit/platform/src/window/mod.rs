mod decorations;
mod dispatch;
mod error;
mod gpu;
mod layer;
mod menus;
mod shm;

use std::cell::RefCell;
use std::io;
use std::os::unix::net::UnixStream;
use std::rc::{Rc, Weak};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use bande_protocols::client::xdg_toplevel_session_v1::XdgToplevelSessionV1;

use crate::app_connection::WindowInSession;
use accesskit::{ActionRequest, TreeUpdate};
use bande_graphics::{
    Appearance, Captured, CpuRenderer, Cursor, Event, PLACEMENTS_AFTER_HOLD, PLACEMENTS_AFTER_REST,
    Point, Rect, Region, ResizeEdge, Scene, SurfaceId, TextSize, TextSystem, damage, pixels,
    placed_surfaces,
};
use bande_protocols::client::bande_shell_v1::{
    BrightnessKey, Edge as DisplayEdge, LauncherMode, MediaKey, Panel, VolumeKey,
};
use bande_widgets::{CaptureRequest, FrameTime, Ui};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, EventQueue, Proxy};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols::xdg::dialog::v1::client::xdg_wm_dialog_v1::XdgWmDialogV1;
use wayland_protocols::xdg::foreign::zv2::client::zxdg_exporter_v2::ZxdgExporterV2;
use wayland_protocols::xdg::foreign::zv2::client::zxdg_importer_v2::ZxdgImporterV2;
use wayland_protocols::xdg::shell::client::xdg_surface::XdgSurface;
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;
use wayland_protocols::xdg::toplevel_tag::v1::client::xdg_toplevel_tag_manager_v1::XdgToplevelTagManagerV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

pub use self::error::WindowError;
use self::gpu::{Gpu, start_gpu};
pub use self::gpu::{RENDERER_VARIABLE, skip_absent_vulkan_drivers};
pub use self::layer::{Edges, KeyboardFocus, LayerKind, LayerOptions};
use self::layer::{anchor_of, keyboard_interactivity, layer_of};
pub(crate) use self::shm::{ShmTarget, Target};
use crate::access::Accessibility;
use crate::appearance::AppearanceWatch;
use crate::capture::Capture;
use crate::chrome::{
    Chrome, Frame, FrameRegions, Header, Look, RESIZE_MARGIN, SHEET_INSET, WINDOW_HEADER_HEIGHT,
};
use crate::clipboard::Clipboard;
use crate::clipboard::{DragAction, DragEvent, DragIcon, Transfer};
use crate::external::External;
use crate::lock::{Locked, SessionLock};
use crate::popup::{Fading, MenuPlacement, Menus};
use crate::presentation::Presentation;
use crate::scale::{Scaling, SurfaceScale};
use crate::seat::Input;
use crate::shell::{PANEL_SINCE, POPOVER_SINCE, Shell};
use crate::text_input::TextInput;

pub const ACTIVATION_TOKEN_VARIABLE: &str = "XDG_ACTIVATION_TOKEN";
const TITLE_BYTES_LIMIT: usize = 1024;
const RELEASE_HIDDEN_AFTER: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePath {
    Cpu,
    Gpu,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Toplevel {
        title: String,
        app_id: String,
        tag: String,
        session: Option<WindowInSession>,
    },
    Dialog {
        title: String,
        app_id: String,
        tag: String,
        session: Option<WindowInSession>,
    },
    Utility {
        title: String,
        app_id: String,
        tag: String,
        session: Option<WindowInSession>,
    },
    Panel {
        title: String,
        app_id: String,
        tag: String,
        session: Option<WindowInSession>,
    },
    Popover {
        title: String,
        app_id: String,
        tag: String,
        session: Option<WindowInSession>,
    },
    Layer(LayerOptions),
    Lock,
}

impl Role {
    pub fn toplevel(title: &str, app_id: &str) -> Self {
        Self::Toplevel {
            title: title.to_owned(),
            app_id: app_id.to_owned(),
            tag: String::new(),
            session: None,
        }
    }

    pub fn dialog(title: &str, app_id: &str) -> Self {
        Self::Dialog {
            title: title.to_owned(),
            app_id: app_id.to_owned(),
            tag: String::new(),
            session: None,
        }
    }

    pub fn utility(title: &str, app_id: &str) -> Self {
        Self::Utility {
            title: title.to_owned(),
            app_id: app_id.to_owned(),
            tag: String::new(),
            session: None,
        }
    }

    pub fn panel(title: &str, app_id: &str) -> Self {
        Self::Panel {
            title: title.to_owned(),
            app_id: app_id.to_owned(),
            tag: String::new(),
            session: None,
        }
    }

    pub fn popover(title: &str, app_id: &str) -> Self {
        Self::Popover {
            title: title.to_owned(),
            app_id: app_id.to_owned(),
            tag: String::new(),
            session: None,
        }
    }

    pub fn tagged(mut self, window_tag: &str) -> Self {
        if let Self::Toplevel { tag, .. }
        | Self::Dialog { tag, .. }
        | Self::Utility { tag, .. }
        | Self::Panel { tag, .. }
        | Self::Popover { tag, .. } = &mut self
        {
            *tag = window_tag.to_owned();
        }
        self
    }

    pub fn in_session(mut self, window: Option<WindowInSession>) -> Self {
        if let Self::Toplevel { session, .. }
        | Self::Dialog { session, .. }
        | Self::Utility { session, .. }
        | Self::Panel { session, .. }
        | Self::Popover { session, .. } = &mut self
        {
            *session = window;
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowEvent {
    SpellingAnswered,
    Input(Event),
    Focus(bool),
    CloseRequested,
    QuitRequested,
    ToplevelChanged(ToplevelState),
    MenuAction(String),
    MenuClosed,
    Pasted(String),
    PastedTransfer(Transfer),
    ClipboardChanged,
    Drag(DragEvent),
    ShowLauncher,
    ScrimPressed,
    OpenLauncher {
        mode: LauncherMode,
        text_field: bool,
    },
    CaptureScreen,
    CaptureArea,
    ActiveApp(String),
    ActiveMenus(String),
    ActiveOutput(String),
    AppsChanged,
    VolumeKey {
        key: VolumeKey,
        fine: bool,
    },
    BrightnessKey {
        key: BrightnessKey,
        fine: bool,
        output: String,
    },
    MediaKey(MediaKey),
    TogglePanel(Panel),
    AppResponding {
        pid: i32,
        app_id: String,
        responding: bool,
    },
    AttentionRequested(String),
    OpenTerminal,
    ShowAppMenu {
        app_id: String,
        menu: String,
        anchor: Rect,
    },
    InputSources {
        sources: Vec<String>,
        active: usize,
    },
    Workspaces {
        count: usize,
        active: usize,
    },
    PanelAnchor {
        panel: Panel,
        output: String,
        edge: DisplayEdge,
        rect: Rect,
    },
    FullScreen {
        output: String,
        full_screen: bool,
    },
    Displays(Vec<String>),
    AppsRunning {
        app_id: String,
        count: u32,
    },
    CapsLock(bool),
    Accessibility(ActionRequest),

    AppearanceChanged(Appearance),
}

#[derive(Debug, Clone, PartialEq)]
struct Opaque {
    area: Rect,
    holes: Vec<Rect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToplevelState {
    pub maximized: bool,
    pub fullscreen: bool,
    pub activated: bool,
    pub tiled: bool,
    pub suspended: bool,
}

impl ToplevelState {
    fn floating(self) -> bool {
        !(self.maximized || self.fullscreen || self.tiled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sizing {
    Resizable,
    Fixed,
}

fn resize_protocol(edge: ResizeEdge) -> xdg_toplevel::ResizeEdge {
    match edge {
        ResizeEdge::Top => xdg_toplevel::ResizeEdge::Top,
        ResizeEdge::Bottom => xdg_toplevel::ResizeEdge::Bottom,
        ResizeEdge::Left => xdg_toplevel::ResizeEdge::Left,
        ResizeEdge::Right => xdg_toplevel::ResizeEdge::Right,
        ResizeEdge::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
        ResizeEdge::TopRight => xdg_toplevel::ResizeEdge::TopRight,
        ResizeEdge::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
        ResizeEdge::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) compositor: Option<WlCompositor>,
    pub(crate) shm: Option<WlShm>,
    pub(crate) subcompositor: Option<WlSubcompositor>,
    pub(crate) dmabuf: Option<wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    pub(crate) background_effects: Option<ExtBackgroundEffectManagerV1>,
    pub(crate) wm_base: Option<XdgWmBase>,
    layer_shell: Option<ZwlrLayerShellV1>,
    pub(crate) activation: Option<XdgActivationV1>,
    pub(crate) dmabuf_formats: Vec<(u32, u64)>,
    pub(crate) idle_inhibit_manager: Option<ZwpIdleInhibitManagerV1>,
    toplevel_tag: Option<XdgToplevelTagManagerV1>,
    toplevel_session: Option<XdgToplevelSessionV1>,
    exporter: Option<ZxdgExporterV2>,
    importer: Option<ZxdgImporterV2>,
    dialogs: Option<XdgWmDialogV1>,
    exported_handle: Option<String>,
    pub(crate) toplevel_drag: Option<wayland_protocols::xdg::toplevel_drag::v1::client::xdg_toplevel_drag_manager_v1::XdgToplevelDragManagerV1>,
    pub(crate) seat: Option<WlSeat>,
    pub(crate) data_device_manager: Option<WlDataDeviceManager>,
    pub(crate) cursor_shape_manager: Option<WpCursorShapeManagerV1>,
    configured: bool,
    configure_unapplied: bool,
    suggested_size: Option<(u32, u32)>,
    closed: bool,
    pub(crate) toplevel: ToplevelState,
    frame_pending: bool,
    frame_done: bool,
    pub(crate) events: Vec<WindowEvent>,
    pub(crate) own_surface: Option<WlSurface>,
    pub(crate) popup_surfaces: Vec<WlSurface>,
    pub(crate) popup_input: Vec<(usize, Event)>,
    pub(crate) popups_changed: bool,
    pub(crate) input: Input,
    pub(crate) clipboard: Clipboard,
    pub(crate) shell: Shell,
    pub(crate) lock: SessionLock,
    pub(crate) text_input: TextInput,
    pub(crate) scaling: Scaling,
    pub(crate) presentation: Presentation,
    pub(crate) capture: Capture,
    outputs: Vec<u32>,
}

impl State {
    fn release(&mut self) {
        if let Some(session) = self.toplevel_session.take() {
            session.destroy();
        }
        self.capture.release();
        if let Some(subcompositor) = self.subcompositor.take() {
            subcompositor.destroy();
        }
        if let Some(dmabuf) = self.dmabuf.take() {
            dmabuf.destroy();
        }
        self.clipboard.release();
        self.text_input.release();
        self.input.release();
        self.shell.release();
        if let Some(seat) = self.seat.take()
            && seat.version() >= 5
        {
            seat.release();
        }
        if let Some(manager) = self.cursor_shape_manager.take() {
            manager.destroy();
        }
        if let Some(activation) = self.activation.take() {
            activation.destroy();
        }
        if let Some(manager) = self.toplevel_tag.take() {
            manager.destroy();
        }
        if let Some(manager) = self.toplevel_drag.take() {
            manager.destroy();
        }
        if let Some(exporter) = self.exporter.take() {
            exporter.destroy();
        }
        if let Some(importer) = self.importer.take() {
            importer.destroy();
        }
        if let Some(dialogs) = self.dialogs.take() {
            dialogs.destroy();
        }
        if let Some(wm_base) = self.wm_base.take() {
            wm_base.destroy();
        }
    }

    pub(crate) fn configure(&mut self, width: u32, height: u32) {
        self.configured = true;
        self.configure_unapplied = true;
        if width > 0 && height > 0 {
            self.suggested_size = Some((width, height));
        }
    }
}

enum ShellSurface {
    Toplevel(XdgSurface, XdgToplevel),
    Layer(ZwlrLayerSurfaceV1),
    Lock(Locked),
}

fn for_role(role: &ShellSurface, appearance: Appearance) -> Appearance {
    match role {
        ShellSurface::Toplevel(..) => appearance,
        ShellSurface::Layer(_) | ShellSurface::Lock(_) => Appearance {
            text_size: TextSize::DEFAULT,
            ..appearance
        },
    }
}

pub struct Window {
    gpu: Gpu,
    pub(crate) cpu: CpuRenderer,
    shm_target: ShmTarget,
    role: ShellSurface,
    pub(crate) surface: WlSurface,
    pub(crate) queue: EventQueue<State>,
    pub(crate) state: State,
    pub(crate) connection: Connection,
    size: (u32, u32),
    surface_scale: Option<SurfaceScale>,
    drawn_scale: f64,
    visible: bool,
    hidden_at: Option<Instant>,
    shown_scene: Option<(Scene, f64)>,
    needs_redraw: bool,
    chrome: Option<Chrome>,
    menus: Option<Menus>,
    fading: Option<Fading>,
    ui_deadline: Option<Instant>,
    window_menu: bool,
    keep_on_top: bool,
    text: Option<TextSystem>,
    shown_frame: Option<Frame>,
    planned: Option<FrameTime>,
    shown_opaque: Option<Opaque>,
    shown_backdrop: Option<Rect>,
    backdrop_effect: Option<ExtBackgroundEffectSurfaceV1>,
    pub(crate) externals: Vec<Weak<RefCell<External>>>,
    staged: bool,
    pub(crate) app_cursor: Cursor,
    pub(crate) accessibility: Accessibility,
    pub(crate) appearance: Rc<RefCell<AppearanceWatch>>,
}

fn fix_size(toplevel: &XdgToplevel, (width, height): (u32, u32)) {
    let (width, height) = (width as i32, height as i32);
    toplevel.set_min_size(width, height);
    toplevel.set_max_size(width, height);
}

impl Window {
    pub fn open(role: Role, size: (u32, u32)) -> Result<Self, WindowError> {
        Self::on(
            Connection::connect_to_env().map_err(WindowError::Connect)?,
            role,
            size,
            None,
            crate::lock::wanted_output(),
        )
    }

    pub fn open_on(stream: UnixStream, role: Role, size: (u32, u32)) -> Result<Self, WindowError> {
        Self::on(
            Connection::from_socket(stream).map_err(WindowError::Connect)?,
            role,
            size,
            None,
            crate::lock::wanted_output(),
        )
    }

    pub fn open_beside(&self, role: Role, size: (u32, u32)) -> Result<Self, WindowError> {
        Self::on(
            self.connection.clone(),
            role,
            size,
            Some(self.appearance.clone()),
            crate::lock::wanted_output(),
        )
    }

    pub fn open_beside_on(
        &self,
        output: &str,
        role: Role,
        size: (u32, u32),
    ) -> Result<Self, WindowError> {
        Self::on(
            self.connection.clone(),
            role,
            size,
            Some(self.appearance.clone()),
            Some(output.to_owned()),
        )
    }

    pub(crate) fn on(
        connection: Connection,
        role: Role,
        size: (u32, u32),
        shared_appearance: Option<Rc<RefCell<AppearanceWatch>>>,
        output: Option<String>,
    ) -> Result<Self, WindowError> {
        crate::spelling::start();
        let fresh = shared_appearance.is_none();
        let appearance = match shared_appearance {
            Some(appearance) => appearance,
            None => Rc::new(RefCell::new(AppearanceWatch::start()?)),
        };
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut state = State::default();
        state.lock.wanted_name = output;
        queue.roundtrip(&mut state)?;
        let compositor = state
            .compositor
            .clone()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        state.shm.as_ref().ok_or(WindowError::Missing("wl_shm"))?;
        state.join_data_device(&handle);
        state.join_primary_selection(&handle);

        let surface = compositor.create_surface(&handle, ());
        state.own_surface = Some(surface.clone());
        state.join_text_input(&surface, &handle);
        let surface_scale = state.scaling.follow(&surface, &handle);
        if surface_scale.is_some() {
            queue.roundtrip(&mut state)?;
        }
        let mut chrome = None;
        let panel = matches!(role, Role::Panel { .. });
        let popover = matches!(role, Role::Popover { .. });
        let (titled, fixed) = match &role {
            Role::Dialog { .. } | Role::Popover { .. } => (false, true),
            Role::Utility { .. } => (true, true),
            Role::Panel { .. } => (false, false),
            Role::Toplevel { .. } | Role::Layer(_) | Role::Lock => (true, false),
        };
        let role = match role {
            Role::Toplevel {
                title,
                app_id,
                tag,
                session,
            }
            | Role::Dialog {
                title,
                app_id,
                tag,
                session,
            }
            | Role::Utility {
                title,
                app_id,
                tag,
                session,
            }
            | Role::Panel {
                title,
                app_id,
                tag,
                session,
            }
            | Role::Popover {
                title,
                app_id,
                tag,
                session,
            } => {
                let wm_base = state
                    .wm_base
                    .clone()
                    .ok_or(WindowError::Missing("xdg_wm_base"))?;
                let xdg_surface = wm_base.get_xdg_surface(&surface, &handle, ());
                let toplevel = xdg_surface.get_toplevel(&handle, ());
                let title = &title[..title.floor_char_boundary(TITLE_BYTES_LIMIT)];
                toplevel.set_title(title.to_owned());
                toplevel.set_app_id(app_id);
                if let Some(app) = state
                    .shell
                    .app
                    .as_ref()
                    .filter(|app| panel && app.version() >= PANEL_SINCE)
                {
                    app.set_panel(&toplevel);
                }
                if let Some(app) = state
                    .shell
                    .app
                    .as_ref()
                    .filter(|app| popover && app.version() >= POPOVER_SINCE)
                {
                    app.set_popover(&toplevel);
                }
                if let Some(manager) = state.toplevel_tag.take() {
                    if !tag.is_empty() {
                        manager.set_toplevel_tag(&toplevel, tag);
                    }
                    manager.destroy();
                }
                state.toplevel_session = session.map(|window| match window.restoring {
                    true => window
                        .session
                        .restore_toplevel(&toplevel, window.name, &handle, ()),
                    false => window
                        .session
                        .add_toplevel(&toplevel, window.name, &handle, ()),
                });
                if fixed {
                    let (width, height) = match titled {
                        true => (
                            size.0 + 2 * SHEET_INSET,
                            size.1 + WINDOW_HEADER_HEIGHT + SHEET_INSET,
                        ),
                        false => size,
                    };
                    fix_size(&toplevel, (width, height));
                }
                chrome = Some(Chrome::new(title.to_owned(), titled, fixed));
                ShellSurface::Toplevel(xdg_surface, toplevel)
            }
            Role::Layer(options) => {
                let layer_shell = state
                    .layer_shell
                    .clone()
                    .ok_or(WindowError::Missing("zwlr_layer_shell_v1"))?;
                let layer = layer_of(options.layer);
                if state.lock.wanted_name.is_some() {
                    queue.roundtrip(&mut state)?;
                }
                let output = state.lock.wanted();
                if state.lock.wanted_name.is_some() && output.is_none() {
                    surface.destroy();
                    return Err(WindowError::Missing("the display"));
                }
                let layer_surface = layer_shell.get_layer_surface(
                    &surface,
                    output.as_ref(),
                    layer,
                    options.namespace,
                    &handle,
                    (),
                );
                layer_surface.set_anchor(anchor_of(options.anchor));
                let stretched = |a: bool, b: bool, length: u32| if a && b { 0 } else { length };
                layer_surface.set_size(
                    stretched(options.anchor.left, options.anchor.right, size.0),
                    stretched(options.anchor.top, options.anchor.bottom, size.1),
                );
                layer_surface.set_exclusive_zone(options.exclusive_zone);
                layer_surface.set_keyboard_interactivity(keyboard_interactivity(options.keyboard));
                ShellSurface::Layer(layer_surface)
            }
            Role::Lock => ShellSurface::Lock(Locked::lock(&mut state, &mut queue, &surface)?),
        };
        if let (Some(activation), Ok(token)) = (
            state.activation.as_ref(),
            std::env::var(ACTIVATION_TOKEN_VARIABLE),
        ) {
            activation.activate(token, &surface);
        }
        if !matches!(role, ShellSurface::Lock(_)) {
            surface.commit();
        }
        while !state.configured {
            queue.blocking_dispatch(&mut state)?;
        }

        let (extra_width, extra_height) = if chrome.as_ref().is_some_and(|chrome| chrome.titled) {
            (2 * SHEET_INSET, WINDOW_HEADER_HEIGHT + SHEET_INSET)
        } else {
            (0, 0)
        };
        let size = state.suggested_size.unwrap_or((
            size.0.saturating_add(extra_width),
            size.1.saturating_add(extra_height),
        ));
        let scale = match surface_scale {
            Some(_) => state.scaling.scale(),
            None => 1.0,
        };
        let gpu = start_gpu(
            &connection,
            &surface,
            (pixels(size.0, scale), pixels(size.1, scale)),
        );
        if fresh {
            appearance.borrow_mut().wait_for_first_reading();
        }
        let current = for_role(&role, appearance.borrow().current());
        state.events.push(WindowEvent::AppearanceChanged(current));
        Ok(Self {
            gpu,
            cpu: CpuRenderer::new(),
            shm_target: ShmTarget::default(),
            role,
            surface,
            queue,
            state,
            connection,
            size,
            surface_scale,
            drawn_scale: 1.0,
            visible: true,
            hidden_at: None,
            shown_scene: None,
            needs_redraw: true,
            chrome,
            menus: None,
            fading: None,
            ui_deadline: None,
            window_menu: false,
            keep_on_top: false,
            text: None,
            shown_frame: None,
            planned: None,
            shown_opaque: None,
            shown_backdrop: None,
            backdrop_effect: None,
            externals: Vec::new(),
            staged: false,
            app_cursor: Cursor::Default,
            accessibility: Accessibility::new()?,
            appearance,
        })
    }

    pub fn unlock(&mut self) -> Result<(), WindowError> {
        if let ShellSurface::Lock(locked) = &self.role {
            locked.unlock();
        }
        self.flush()
    }

    pub fn size(&self) -> (u32, u32) {
        self.frame().content_size()
    }

    pub fn set_frame_regions(&mut self, regions: FrameRegions) {
        if let Some(chrome) = &mut self.chrome
            && chrome.regions != Some(regions)
        {
            chrome.regions = Some(regions);
            self.needs_redraw = true;
        }
    }

    pub fn set_header(&mut self, header: Header) {
        if let Some(chrome) = &mut self.chrome
            && chrome.header.as_ref() != Some(&header)
        {
            chrome.header = Some(header);
            self.needs_redraw = true;
        }
    }

    pub fn set_move_areas(&mut self, areas: Vec<Rect>) {
        if let Some(chrome) = &mut self.chrome {
            chrome.move_areas = areas;
        }
    }

    pub fn set_detachable_tabs(&mut self, detachable: bool) {
        if let Some(chrome) = &mut self.chrome {
            chrome.detachable_tabs = detachable;
        }
    }

    pub fn set_menu_button(&mut self, shown: bool) {
        if let Some(chrome) = &mut self.chrome
            && chrome.titled
            && chrome.menu != shown
        {
            chrome.menu = shown;
            self.needs_redraw = true;
            self.announce_menu_button();
        }
    }

    pub fn header_height(&self) -> u32 {
        self.title_height()
    }

    pub(crate) fn content_origin(&self) -> Point {
        match &self.chrome {
            Some(_) => self.frame().content_origin(),
            None => Point::default(),
        }
    }

    pub(crate) fn content_offset(&self) -> Point {
        let frame = self.frame();
        let origin = frame.content_origin();
        Point::new(origin.x - frame.margin, origin.y - frame.margin)
    }

    fn keep_fixed_size(&self, size: (u32, u32)) {
        if let ShellSurface::Toplevel(_, toplevel) = &self.role
            && self.chrome.as_ref().is_some_and(|chrome| chrome.fixed)
        {
            fix_size(toplevel, size);
        }
    }

    pub fn is_resizable(&self) -> bool {
        self.chrome.as_ref().is_some_and(|chrome| !chrome.fixed)
    }

    fn geometry(&self) -> (u32, u32) {
        let keeps_own_size = self.chrome.as_ref().is_some_and(|chrome| chrome.fixed)
            && self.state.toplevel.floating();
        match self.state.suggested_size {
            Some(suggested) if !keeps_own_size => suggested,
            _ => self.size,
        }
    }

    pub fn resize_content(&mut self, size: (u32, u32), sizing: Sizing) {
        let (ShellSurface::Toplevel(_, toplevel), Some(chrome)) = (&self.role, &mut self.chrome)
        else {
            return;
        };
        chrome.fixed = sizing == Sizing::Fixed;
        self.needs_redraw = true;
        if sizing == Sizing::Resizable {
            toplevel.set_min_size(0, 0);
            toplevel.set_max_size(0, 0);
            if !self.state.toplevel.floating() {
                return;
            }
        }
        let (extra_width, extra_height) = self.frame().chrome_size();
        let frame = (size.0 + extra_width, size.1 + extra_height);
        self.state.suggested_size = None;
        self.size = frame;
        self.keep_fixed_size(frame);
    }

    fn title_height(&self) -> u32 {
        match &self.chrome {
            Some(chrome) if chrome.titled && !self.state.toplevel.fullscreen => {
                WINDOW_HEADER_HEIGHT
            }
            _ => 0,
        }
    }

    fn resizes_from_band(&self) -> bool {
        let toplevel = self.state.toplevel;
        self.is_resizable() && !toplevel.maximized && !toplevel.fullscreen
    }

    fn resize_margin(&self) -> u32 {
        let transparent = match &self.gpu {
            Gpu::Ready(gpu) => gpu.transparent,
            Gpu::Starting(_) | Gpu::Released | Gpu::Unavailable => true,
        };
        if self.resizes_from_band() && transparent {
            RESIZE_MARGIN
        } else {
            0
        }
    }

    pub(crate) fn frame(&self) -> Frame {
        let (width, height) = self.geometry();
        let title = self.title_height();
        Frame {
            margin: self.resize_margin() as f32,
            resizes: self.resizes_from_band(),
            title: title as f32,
            inset: if title > 0 { SHEET_INSET as f32 } else { 0.0 },
            regions: self.chrome.as_ref().and_then(|chrome| chrome.regions),
            header: self
                .chrome
                .as_ref()
                .is_some_and(|chrome| chrome.header.is_some()),
            menu: self.chrome.as_ref().is_some_and(|chrome| chrome.menu),
            width: width as f32,
            height: height as f32,
        }
    }

    pub fn export(&mut self) {
        if let Some(exporter) = self.state.exporter.as_ref() {
            exporter.export_toplevel(&self.surface, &self.queue.handle(), ());
        }
    }

    pub fn exported_handle(&self) -> Option<&str> {
        self.state.exported_handle.as_deref()
    }

    pub fn attach_to(&mut self, parent_handle: &str) {
        let ShellSurface::Toplevel(_, toplevel) = &self.role else {
            return;
        };
        if let Some(importer) = self.state.importer.as_ref() {
            let imported =
                importer.import_toplevel(parent_handle.to_owned(), &self.queue.handle(), ());
            imported.set_parent_of(&self.surface);
        }
        if let Some(dialogs) = self.state.dialogs.as_ref() {
            dialogs
                .get_xdg_dialog(toplevel, &self.queue.handle(), ())
                .set_modal();
        }
    }

    pub fn set_title(&mut self, title: &str) {
        let title = &title[..title.floor_char_boundary(TITLE_BYTES_LIMIT)];
        if let ShellSurface::Toplevel(_, toplevel) = &self.role {
            toplevel.set_title(title.to_owned());
        }
        if let Some(chrome) = &mut self.chrome {
            chrome.title = title.to_owned();
            self.needs_redraw = true;
        }
    }

    pub fn share_fonts(&mut self, text: &TextSystem) {
        self.text = Some(text.sharing_fonts());
    }

    pub fn appearance(&self) -> Appearance {
        self.for_role(self.appearance.borrow().current())
    }

    pub(crate) fn for_role(&self, appearance: Appearance) -> Appearance {
        for_role(&self.role, appearance)
    }

    pub fn connection(&self) -> Connection {
        self.connection.clone()
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed
    }

    pub fn close(&mut self) {
        self.state.closed = true;
    }

    pub fn request_close(&mut self) {
        self.state.events.push(WindowEvent::CloseRequested);
    }

    pub fn toplevel_state(&self) -> ToplevelState {
        self.state.toplevel
    }

    pub fn toggle_full_screen(&mut self) -> Result<(), WindowError> {
        let toplevel = self.toplevel()?;
        if self.state.toplevel.fullscreen {
            toplevel.unset_fullscreen();
        } else if self.is_resizable() {
            toplevel.set_fullscreen(None);
        }
        self.flush()
    }

    pub(crate) fn toplevel(&self) -> Result<&XdgToplevel, WindowError> {
        match &self.role {
            ShellSurface::Toplevel(_, toplevel) => Ok(toplevel),
            _ => Err(WindowError::Missing("a toplevel surface")),
        }
    }

    pub(crate) fn grab(&self) -> Result<(&WlSeat, u32), WindowError> {
        let seat = self
            .state
            .seat
            .as_ref()
            .ok_or(WindowError::Missing("wl_seat"))?;
        let serial = self
            .state
            .input
            .serial
            .ok_or(WindowError::Missing("an input serial"))?;
        Ok((seat, serial))
    }

    pub fn start_move(&mut self) -> Result<(), WindowError> {
        let (seat, serial) = self.grab()?;
        self.toplevel()?._move(seat, serial);
        self.flush()
    }

    pub fn start_resize(&mut self, edge: ResizeEdge) -> Result<(), WindowError> {
        let (seat, serial) = self.grab()?;
        self.toplevel()?.resize(seat, serial, resize_protocol(edge));
        self.flush()
    }

    pub fn minimize(&mut self) -> Result<(), WindowError> {
        self.toplevel()?.set_minimized();
        self.flush()
    }

    pub fn toggle_zoom(&mut self) -> Result<(), WindowError> {
        if !self.is_resizable() {
            return Ok(());
        }
        let toplevel = self.toplevel()?;
        if self.state.toplevel.maximized {
            toplevel.unset_maximized();
        } else {
            toplevel.set_maximized();
        }
        self.flush()
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn request_redraw(&mut self) {
        self.needs_redraw = true;
    }

    pub(crate) fn stage(&mut self) {
        self.staged = true;
        self.needs_redraw = true;
    }

    pub(crate) fn wants_redraw(&self) -> bool {
        self.needs_redraw
    }

    pub fn frame_time(&self) -> FrameTime {
        self.state
            .presentation
            .frame_time(Instant::now(), self.state.frame_pending)
    }

    pub(crate) fn plan_frame(&mut self) -> FrameTime {
        let frame = self.frame_time();
        self.planned = Some(frame);
        frame
    }

    pub(crate) fn wants_frame(&self) -> bool {
        self.visible
            && !self.state.frame_pending
            && (self.needs_redraw || self.resized() || self.scale() != self.drawn_scale)
    }

    pub fn scale(&self) -> f64 {
        match self.surface_scale {
            Some(_) => self.state.scaling.scale(),
            None => 1.0,
        }
    }

    fn resized(&self) -> bool {
        self.state
            .suggested_size
            .is_some_and(|size| size != self.size)
    }

    pub fn wait_for_gpu(&mut self) -> bool {
        if let Gpu::Starting(receiver) = &self.gpu {
            self.gpu = match receiver.recv() {
                Ok(Ok(gpu)) => Gpu::Ready(Box::new(gpu)),
                Ok(Err(_)) | Err(_) => Gpu::Unavailable,
            };
        }
        matches!(self.gpu, Gpu::Ready(_))
    }

    pub fn wait_for_frame(&mut self) -> Result<(), WindowError> {
        while self.state.frame_pending && !self.state.closed {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        Ok(())
    }

    pub fn dispatch(&mut self) -> Result<(), WindowError> {
        self.queue.blocking_dispatch(&mut self.state)?;
        self.release_uploads();
        Ok(())
    }

    fn release_uploads(&mut self) {
        if std::mem::take(&mut self.state.frame_done)
            && let Gpu::Ready(gpu) = &mut self.gpu
        {
            gpu.release_uploads();
        }
    }

    pub(crate) fn has_events(&self) -> bool {
        !self.state.events.is_empty()
    }

    pub(crate) fn dispatch_pending(&mut self) -> Result<(), WindowError> {
        self.queue.dispatch_pending(&mut self.state)?;
        self.release_uploads();
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> Result<(), WindowError> {
        match self.connection.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn take_window_tiled(&mut self) {
        let window_tiled = std::mem::take(&mut self.state.shell.window_tiled);
        let Ok(own) = self.toplevel().cloned() else {
            return;
        };
        for (toplevel, tiled) in window_tiled {
            if toplevel == own && self.state.toplevel.tiled != tiled {
                self.state.toplevel.tiled = tiled;
                self.state
                    .events
                    .push(WindowEvent::ToplevelChanged(self.state.toplevel));
            }
        }
    }

    pub fn take_events(&mut self) -> Vec<WindowEvent> {
        self.take_arrival();
        self.take_drop();
        self.serve_menus(self.frame_time());
        let menus_open = std::mem::take(&mut self.state.shell.menus_open);
        if let (Ok(own), Some(chrome)) = (self.toplevel().cloned(), &mut self.chrome) {
            for (toplevel, open) in menus_open {
                if toplevel == own && chrome.menu_open != open {
                    chrome.menu_open = open;
                    self.needs_redraw = true;
                }
            }
        }
        self.take_window_tiled();
        let events = std::mem::take(&mut self.state.events);
        if self.chrome.is_none() {
            return events;
        }
        let mut passed = Vec::with_capacity(events.len());
        for event in events {
            self.through_chrome(event, &mut passed);
        }
        passed
    }

    pub fn drive(
        &mut self,
        ui: &mut Ui,
        event: WindowEvent,
    ) -> Result<Option<WindowEvent>, WindowError> {
        ui.context().display_scale = self.scale();
        let (changed, unhandled) = match event {
            WindowEvent::Input(input) => {
                let changed = ui.handle(&input);
                if let Some((menu, at)) = ui.take_menu_request() {
                    self.show_menu(menu, MenuPlacement::AtPoint(at))?;
                }
                if let Some(link) = ui.take_link_request()
                    && let Err(error) = bande_xdg::open(&link, None)
                {
                    tracing::warn!("cannot open a link: {error}");
                }
                if let Some(request) = ui.take_clipboard_request() {
                    self.serve_clipboard(request)?;
                }
                if ui.take_input_method_reset() {
                    self.reset_text_input();
                }
                if let Some((scene, hotspot, text)) = ui.take_text_drag() {
                    self.start_drag(
                        Transfer::text(&text),
                        &[DragAction::Copy, DragAction::Move],
                        Some(DragIcon { scene, hotspot }),
                    )?;
                }
                self.set_cursor(ui.cursor());
                if let Some(text) = ui.take_look_up_request() {
                    self.look_up(text);
                }
                let unhandled = match input {
                    Event::Key { .. } if !changed => Some(WindowEvent::Input(input)),
                    _ => None,
                };
                (changed, unhandled)
            }
            WindowEvent::Drag(drag) => self.drive_drag(ui, drag),

            WindowEvent::Accessibility(request) => match ui.perform(&request) {
                true => (true, None),
                false => (false, Some(WindowEvent::Accessibility(request))),
            },
            WindowEvent::MenuAction(action) => match ui.menu_action(&action) {
                true => {
                    if ui.take_emoji_picker_request() {
                        self.show_emoji_picker()?;
                    }
                    if let Some(text) = ui.take_look_up_request() {
                        self.look_up(text);
                    }
                    (true, None)
                }
                false => (false, Some(WindowEvent::MenuAction(action))),
            },
            WindowEvent::Pasted(text) => match ui.pasted(&text) {
                true => (true, None),
                false => (false, Some(WindowEvent::Pasted(text))),
            },
            WindowEvent::MenuClosed => match ui.menu_closed() {
                true => (false, None),
                false => (false, Some(WindowEvent::MenuClosed)),
            },
            WindowEvent::AppearanceChanged(appearance) => {
                ui.set_appearance(appearance);
                (true, Some(WindowEvent::AppearanceChanged(appearance)))
            }
            WindowEvent::SpellingAnswered => {
                let edited = ui.spelling_answered();
                let menu = ui.take_menu_request();
                let shown = menu.is_some();
                if let Some((menu, at)) = menu {
                    self.show_menu(menu, MenuPlacement::AtPoint(at))?;
                }
                (edited || shown, None)
            }
            WindowEvent::CapsLock(on) => {
                ui.context().caps_lock = on;
                (true, Some(WindowEvent::CapsLock(on)))
            }
            event => (false, Some(event)),
        };
        match ui.take_capture_request() {
            Some(CaptureRequest::Start { area, pointer }) => {
                if let Err(error) = self.capture_input(area, pointer) {
                    tracing::warn!(%error, "cannot capture the keyboard and pointer");
                    ui.handle(&Event::Captured(Captured::Ended));
                }
            }
            Some(CaptureRequest::End) => self.release_input()?,
            None => {}
        }
        if changed || ui.is_animating() {
            self.request_redraw();
        }
        self.ui_deadline = ui.deadline();
        Ok(unhandled)
    }

    fn drive_drag(&mut self, ui: &mut Ui, drag: DragEvent) -> (bool, Option<WindowEvent>) {
        let was_target = ui.is_drop_target();
        match &drag {
            DragEvent::Entered { position, .. } | DragEvent::Moved(position)
                if self.drag_offers_text() =>
            {
                match ui.drag_over(Some(*position)) {
                    Some(action) => {
                        self.accept_drag(Some(action));
                        (true, None)
                    }
                    None => {
                        if was_target {
                            self.accept_drag(None);
                        }
                        (was_target, Some(WindowEvent::Drag(drag)))
                    }
                }
            }
            DragEvent::Left => {
                ui.drag_over(None);
                (was_target, Some(WindowEvent::Drag(drag)))
            }
            DragEvent::Dropped {
                position,
                transfer,
                action: Some(action),
            } if was_target => (ui.dropped(*position, &transfer.text, *action), None),
            DragEvent::Finished(action) if ui.drag_finished(*action) => (true, None),
            _ => (false, Some(WindowEvent::Drag(drag))),
        }
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.menus
            .as_ref()
            .and_then(Menus::deadline)
            .into_iter()
            .chain(self.ui_deadline)
            .chain(self.release_deadline())
            .chain(self.drag_deadline())
            .chain(self.placements_deadline())
            .min()
    }

    fn placements_deadline(&self) -> Option<Instant> {
        let wait = self.chrome.as_ref()?.zoom_wait?;
        Some(
            wait.since
                + match wait.held {
                    true => PLACEMENTS_AFTER_HOLD,
                    false => PLACEMENTS_AFTER_REST,
                },
        )
    }

    fn release_deadline(&self) -> Option<Instant> {
        let holding =
            !matches!(self.gpu, Gpu::Released | Gpu::Unavailable) || self.shm_target.is_holding();
        self.hidden_at
            .filter(|_| holding)
            .map(|hidden_at| hidden_at + RELEASE_HIDDEN_AFTER)
    }

    fn release_rendering(&mut self) {
        self.wait_for_gpu();
        if matches!(self.gpu, Gpu::Ready(_)) {
            self.gpu = Gpu::Released;
        }
        self.shm_target.destroy();
        self.cpu = CpuRenderer::new();
    }

    pub(crate) fn wake_ui_at(&mut self, wake: Option<Instant>) {
        if wake.is_some() {
            self.ui_deadline = self.ui_deadline.into_iter().chain(wake).min();
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) {
        if self
            .release_deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.release_rendering();
        }
        self.serve_menus(self.frame_time());
        self.abandon_unfinished_drag(now);
        if self
            .placements_deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.offer_placements();
        }
        if self.ui_deadline.is_some_and(|deadline| deadline <= now) {
            self.ui_deadline = None;
            self.request_redraw();
        }
    }

    pub(crate) fn pointer_in_content(&self) -> bool {
        self.chrome
            .as_ref()
            .is_none_or(|chrome| chrome.pointer_in_content)
    }

    pub(crate) fn publish_accessibility(&mut self, tree: TreeUpdate) {
        let mut tree = match &self.chrome {
            Some(chrome) => chrome.wrap_tree(&self.frame(), tree),
            None => tree,
        };
        if let Some(menus) = &self.menus
            && let Some(root) = tree.tree.as_ref().map(|info| info.root)
        {
            let (nodes, roots, focus) = menus.tracker().access();
            if let Some((_, node)) = tree.nodes.iter_mut().find(|(id, _)| *id == root) {
                for menu in roots {
                    node.push_child(menu);
                }
            }
            tree.nodes.extend(nodes);
            if let Some(focus) = focus {
                tree.focus = focus;
            }
        }
        self.accessibility.publish(tree);
    }

    pub(crate) fn perform_window_action(&mut self, request: &ActionRequest) -> bool {
        let item = self
            .menus
            .as_ref()
            .and_then(|menus| menus.tracker().item_for_node(request.target_node));
        let frame = self.frame_time();
        if let (Some((level, index)), Some(menus)) = (item, &mut self.menus) {
            let outcome = menus.activate(level, index, frame.on_screen());
            if self.apply_menu_outcome(outcome, true)
                && let Err(error) = self.draw_menus(frame)
            {
                tracing::warn!(%error, "cannot draw the menus");
            }
            return true;
        }
        self.perform_chrome(request)
    }

    fn framed(&mut self, frame: &Frame, scene: Scene, presentation: Instant) -> Scene {
        let appearance = self.appearance.borrow().current();
        let toplevel = self.state.toplevel;
        let look = Look {
            active: toplevel.activated,
            zoomed: toplevel.maximized || toplevel.fullscreen,
            fullscreen: toplevel.fullscreen,
        };
        match &mut self.chrome {
            Some(chrome) => {
                chrome.frame_scene(frame, scene, &appearance.palette(), look, presentation)
            }
            None => scene,
        }
    }

    fn declare_opaque(&mut self, opaque: Option<Opaque>) {
        if self.shown_opaque == opaque {
            return;
        }
        let Some(compositor) = &self.state.compositor else {
            return;
        };
        let Some(Opaque { area, holes }) = &opaque else {
            self.shown_opaque = None;
            self.surface.set_opaque_region(None);
            return;
        };
        let region = compositor.create_region(&self.queue.handle(), ());
        let left = area.x.ceil();
        let top = area.y.ceil();
        region.add(
            left as i32,
            top as i32,
            ((area.x + area.width).floor() - left) as i32,
            ((area.y + area.height).floor() - top) as i32,
        );
        for hole in holes {
            let left = hole.x.floor();
            let top = hole.y.floor();
            region.subtract(
                left as i32,
                top as i32,
                ((hole.x + hole.width).ceil() - left) as i32,
                ((hole.y + hole.height).ceil() - top) as i32,
            );
        }
        self.surface.set_opaque_region(Some(&region));
        region.destroy();
        self.shown_opaque = opaque;
    }

    fn externals_below(&self) -> bool {
        match &self.gpu {
            Gpu::Ready(gpu) => gpu.transparent,
            Gpu::Starting(_) | Gpu::Released | Gpu::Unavailable => true,
        }
    }

    pub fn set_backdrop(&mut self, area: Option<Rect>) {
        if self.shown_backdrop == area {
            return;
        }
        let handle = self.queue.handle();
        let (Some(manager), Some(compositor)) =
            (&self.state.background_effects, &self.state.compositor)
        else {
            return;
        };
        self.shown_backdrop = area;
        let effect = self
            .backdrop_effect
            .get_or_insert_with(|| manager.get_background_effect(&self.surface, &handle, ()));
        let region = area.map(|area| {
            let region = compositor.create_region(&handle, ());
            let left = area.x.floor();
            let top = area.y.floor();
            region.add(
                left as i32,
                top as i32,
                ((area.x + area.width).ceil() - left) as i32,
                ((area.y + area.height).ceil() - top) as i32,
            );
            region
        });
        effect.set_blur_region(region.as_ref());
        if let Some(region) = region {
            region.destroy();
        }
    }

    fn frame_surface(&mut self, frame: &Frame) {
        if self.shown_frame.as_ref() == Some(frame) {
            return;
        }
        self.announce_menu_button();
        self.shown_frame = Some(*frame);
        let ShellSurface::Toplevel(xdg_surface, _) = &self.role else {
            return;
        };
        let whole = |value: f32| value.round() as i32;
        xdg_surface.set_window_geometry(
            whole(frame.margin),
            whole(frame.margin),
            whole(frame.width),
            whole(frame.height),
        );
        if frame.margin == 0.0 {
            self.surface.set_input_region(None);
            return;
        }
        let Some(compositor) = &self.state.compositor else {
            return;
        };
        let region = compositor.create_region(&self.queue.handle(), ());
        let area = frame.input_area();
        region.add(
            whole(area.x),
            whole(area.y),
            whole(area.width),
            whole(area.height),
        );
        self.surface.set_input_region(Some(&region));
        region.destroy();
    }

    pub fn hide(&mut self) -> Result<(), WindowError> {
        if !self.visible {
            return Ok(());
        }
        self.visible = false;
        self.hidden_at = Some(Instant::now());
        self.shown_scene = None;
        self.surface.attach(None, 0, 0);
        self.surface.commit();
        self.state.frame_pending = false;
        self.flush()
    }

    pub fn show(&mut self) -> Result<(), WindowError> {
        if self.visible {
            return Ok(());
        }
        self.state.configured = false;
        self.surface.commit();
        while !self.state.configured {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        self.visible = true;
        self.hidden_at = None;
        if matches!(self.gpu, Gpu::Released) {
            let size = self.geometry();
            let scale = self.scale();
            self.gpu = start_gpu(
                &self.connection,
                &self.surface,
                (pixels(size.0, scale), pixels(size.1, scale)),
            );
        }
        self.needs_redraw = true;
        Ok(())
    }

    pub fn draw(&mut self, scene: Scene) -> Result<FramePath, WindowError> {
        self.wait_for_frame()?;
        self.queue.dispatch_pending(&mut self.state)?;
        self.poll_gpu();
        let frame_time = match self.planned.take() {
            Some(planned) => planned,
            None => self.frame_time(),
        };
        self.state.suggested_size = None;
        let (extra_width, extra_height) = self.frame().chrome_size();
        let size = (scene.width + extra_width, scene.height + extra_height);
        if size != self.size {
            self.keep_fixed_size(size);
        }
        self.size = size;
        let surface_state = (self.shown_frame, self.shown_opaque.clone());
        let (scene, opaque) = if self.chrome.is_some() {
            let frame = self.frame();
            let sheet_opaque = scene.background.a == u8::MAX;
            let framed = self.framed(&frame, scene, frame_time.presentation);
            self.frame_surface(&frame);
            let translucent = self.appearance.borrow().current().palette().frame.a != u8::MAX;
            self.set_backdrop(translucent.then(|| frame.window()));
            let opaque = sheet_opaque.then(|| match translucent {
                true => frame.opaque_sheet(),
                false => frame.window(),
            });
            (framed, opaque)
        } else {
            let opaque = (scene.background.a == u8::MAX)
                .then(|| Rect::new(0.0, 0.0, scene.width as f32, scene.height as f32));
            (scene, opaque)
        };
        let scale = self.scale();
        let below = self.externals_below();
        let placed: Vec<(SurfaceId, Rect)> = match self.externals.is_empty() {
            true => Vec::new(),
            false => placed_surfaces(&scene, scale)
                .into_iter()
                .map(|placed| (placed.id, placed.area))
                .collect(),
        };
        let holes = match below {
            true => placed.iter().map(|(_, area)| *area).collect(),
            false => Vec::new(),
        };
        self.declare_opaque(opaque.map(|area| Opaque { area, holes }));
        self.needs_redraw = self.chrome.as_ref().is_some_and(|chrome| chrome.animating);
        let unchanged = !self.state.configure_unapplied
            && !self.staged
            && surface_state == (self.shown_frame, self.shown_opaque.clone())
            && self
                .shown_scene
                .as_ref()
                .is_some_and(|(shown, shown_scale)| *shown_scale == scale && *shown == scene);
        if unchanged {
            return Ok(FramePath::Unchanged);
        }
        self.state.configure_unapplied = false;
        self.staged = false;
        let handle = self.queue.handle();
        self.surface.frame(&handle, ());
        self.state
            .presentation
            .committing(&self.surface, &handle, frame_time);
        self.state.frame_pending = true;
        self.drawn_scale = scale;
        if let Some(surface_scale) = &self.surface_scale {
            surface_scale.show_at((scene.width, scene.height));
        }
        let damage = self
            .shown_scene
            .take()
            .filter(|(_, shown_scale)| *shown_scale == scale)
            .and_then(|(shown, _)| damage(&shown, &scene, scale));
        if !self.externals.is_empty() {
            self.place_externals(&placed, below);
        }
        let presented = match &mut self.gpu {
            Gpu::Ready(gpu) => Some(gpu.present(&scene, scale, damage.as_deref())),
            Gpu::Starting(_) | Gpu::Released | Gpu::Unavailable => None,
        };
        let path = match presented {
            Some(Ok(())) => {
                if self.shm_target.is_holding() {
                    self.shm_target.destroy();
                    self.cpu = CpuRenderer::new();
                }
                FramePath::Gpu
            }
            Some(Err(error)) => {
                tracing::warn!(%error, "drawing on the CPU after a GPU failure");
                self.gpu = Gpu::Unavailable;
                self.draw_on_cpu(&scene, None)?;
                FramePath::Cpu
            }
            None => {
                self.draw_on_cpu(&scene, damage.as_deref())?;
                FramePath::Cpu
            }
        };
        self.shown_scene = Some((scene, scale));
        self.settle_externals();
        self.flush()?;
        Ok(path)
    }

    pub(crate) fn pace(&mut self) {
        self.surface.frame(&self.queue.handle(), ());
        self.surface.commit();
        self.state.frame_pending = true;
    }

    fn poll_gpu(&mut self) {
        if let Gpu::Starting(receiver) = &self.gpu {
            match receiver.try_recv() {
                Ok(Ok(gpu)) => self.gpu = Gpu::Ready(Box::new(gpu)),
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => self.gpu = Gpu::Unavailable,
                Err(TryRecvError::Empty) => {}
            }
        }
    }

    fn draw_on_cpu(&mut self, scene: &Scene, damage: Option<&[Region]>) -> Result<(), WindowError> {
        let shm = self
            .state
            .shm
            .as_ref()
            .ok_or(WindowError::Missing("wl_shm"))?;
        let handle = self.queue.handle();
        let target = Target {
            shm,
            handle: &handle,
            surface: &self.surface,
        };
        self.shm_target
            .present(&mut self.cpu, target, scene, self.drawn_scale, damage)
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        if let Some(menus) = self.menus.take() {
            menus.destroy(&mut self.state);
        }
        if let Some(fading) = self.fading.take() {
            fading.destroy();
        }
        self.wait_for_gpu();
        self.gpu = Gpu::Unavailable;
        self.shm_target.destroy();
        match &self.role {
            ShellSurface::Toplevel(xdg_surface, toplevel) => {
                toplevel.destroy();
                xdg_surface.destroy();
            }
            ShellSurface::Layer(layer_surface) => layer_surface.destroy(),
            ShellSurface::Lock(_) => {}
        }
        self.surface.destroy();
        self.state.release();
        let _ = self.connection.flush();
    }
}
