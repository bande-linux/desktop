use std::cell::RefCell;
use std::ffi::c_void;
use std::fmt;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use accesskit::{ActionRequest, TreeUpdate};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use rustix::fs::{MemfdFlags, memfd_create};
use unbound_graphics::{
    Appearance, CpuRenderer, Cursor, DrawError, Event, GpuDevice, GpuRenderer, Point, Pointer,
    PointerButton, Rect, Region, ResizeEdge, Scene, TextSize, TextSystem, damage, pixels,
};
use unbound_protocols::client::unbound_shell_v1::{
    BrightnessKey, Edge as DisplayEdge, LauncherMode, MediaKey, Panel, VolumeKey,
};
use unbound_widgets::{FrameTime, Menu, MenuItem, MenuOutcome, Ui};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    ConnectError, Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle,
    delegate_noop,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::ExtForeignToplevelListV1;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{self, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, ZwlrLayerSurfaceV1,
};

use crate::L10N;
use crate::access::Accessibility;
use crate::appearance::AppearanceWatch;
use crate::chrome::{
    Chrome, Control, Frame, FrameRegions, Header, Look, RESIZE_MARGIN, SHEET_INSET,
    WINDOW_HEADER_HEIGHT, Zone, control_for_node,
};
use crate::clipboard::Clipboard;
use crate::clipboard::{DragAction, DragEvent, DragIcon, Transfer};
use crate::lock::{Locked, SessionLock};
use crate::popup::{Fading, MenuPlacement, Menus, Opening, Parent};
use crate::presentation::Presentation;
use crate::scale::{Scaling, SurfaceScale};
use crate::seat::{CLICK_SLOP, Input};
use crate::shell::Shell;
use crate::text_input::TextInput;

pub const RENDERER_VARIABLE: &str = "UNBOUND_RENDERER";
pub const ACTIVATION_TOKEN_VARIABLE: &str = "XDG_ACTIVATION_TOKEN";
const RELEASE_HIDDEN_AFTER: Duration = Duration::from_secs(30);
const TILING_MENU_AFTER: Duration = Duration::from_millis(500);
const MAX_STALE_REGIONS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePath {
    Cpu,
    Gpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Background,
    Bottom,
    Top,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    pub const ALL: Self = Self {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardFocus {
    None,
    OnDemand,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerOptions {
    pub layer: LayerKind,
    pub anchor: Edges,
    pub exclusive_zone: i32,
    pub keyboard: KeyboardFocus,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Toplevel { title: String, app_id: String },
    Layer(LayerOptions),
    Lock,
}

impl Role {
    pub fn toplevel(title: &str, app_id: &str) -> Self {
        Self::Toplevel {
            title: title.to_owned(),
            app_id: app_id.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowEvent {
    Input(Event),
    Focus(bool),
    CloseRequested,
    QuitRequested,
    ToplevelChanged(ToplevelState),
    MenuAction(String),
    MenuClosed,
    Pasted(String),
    PastedTransfer(Transfer),
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
    ActiveOutput(String),
    AppsChanged,
    VolumeKey {
        key: VolumeKey,
        fine: bool,
    },
    BrightnessKey {
        key: BrightnessKey,
        fine: bool,
    },
    MediaKey(MediaKey),
    TogglePanel(Panel),
    AppResponding {
        pid: i32,
        app_id: String,
        responding: bool,
    },
    AttentionRequested(String),
    FocusShell(String),
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

#[derive(Debug)]
pub enum WindowError {
    Connect(ConnectError),
    Dispatch(DispatchError),
    Wayland(WaylandError),
    Missing(&'static str),
    Io(io::Error),
    Render(DrawError),
    LockRefused,
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "cannot connect to the compositor: {error}"),
            Self::Dispatch(error) => write!(f, "Wayland dispatch failed: {error}"),
            Self::Wayland(error) => write!(f, "Wayland connection failed: {error}"),
            Self::Missing(global) => write!(f, "the compositor does not offer {global}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Render(error) => write!(f, "{error}"),
            Self::LockRefused => write!(f, "the compositor refused to lock the session"),
        }
    }
}

impl std::error::Error for WindowError {}

impl From<DispatchError> for WindowError {
    fn from(error: DispatchError) -> Self {
        Self::Dispatch(error)
    }
}

impl From<WaylandError> for WindowError {
    fn from(error: WaylandError) -> Self {
        Self::Wayland(error)
    }
}

impl From<io::Error> for WindowError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<DrawError> for WindowError {
    fn from(error: DrawError) -> Self {
        Self::Render(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToplevelState {
    pub maximized: bool,
    pub fullscreen: bool,
    pub activated: bool,
    pub tiled: bool,
    pub suspended: bool,
}

fn resize_cursor(edge: ResizeEdge) -> Cursor {
    match edge {
        ResizeEdge::Top => Cursor::ResizeTop,
        ResizeEdge::Bottom => Cursor::ResizeBottom,
        ResizeEdge::Left => Cursor::ResizeLeft,
        ResizeEdge::Right => Cursor::ResizeRight,
        ResizeEdge::TopLeft => Cursor::ResizeTopLeft,
        ResizeEdge::TopRight => Cursor::ResizeTopRight,
        ResizeEdge::BottomLeft => Cursor::ResizeBottomLeft,
        ResizeEdge::BottomRight => Cursor::ResizeBottomRight,
    }
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
    pub(crate) wm_base: Option<XdgWmBase>,
    layer_shell: Option<ZwlrLayerShellV1>,
    pub(crate) activation: Option<XdgActivationV1>,
    pub(crate) seat: Option<WlSeat>,
    pub(crate) data_device_manager: Option<WlDataDeviceManager>,
    pub(crate) cursor_shape_manager: Option<WpCursorShapeManagerV1>,
    configured: bool,
    suggested_size: Option<(u32, u32)>,
    closed: bool,
    pub(crate) toplevel: ToplevelState,
    frame_pending: bool,
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
    outputs: Vec<u32>,
}

impl State {
    pub(crate) fn configure(&mut self, width: u32, height: u32) {
        self.configured = true;
        if width > 0 && height > 0 {
            self.suggested_size = Some((width, height));
        }
    }
}

struct ShmBuffer {
    buffer: WlBuffer,
    file: File,
    width: u32,
    height: u32,
    busy: Arc<AtomicBool>,
    stale: Option<Vec<Region>>,
}

struct GpuSurface {
    gpu: Arc<GpuDevice>,
    renderer: GpuRenderer,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    configured: Option<(u32, u32)>,
    transparent: bool,
}

enum Gpu {
    Starting(Receiver<Result<GpuSurface, DrawError>>),
    Ready(Box<GpuSurface>),
    Released,
    Unavailable,
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

fn menu_text(text: &mut Option<TextSystem>, appearance: Appearance) -> &mut TextSystem {
    let text = text.get_or_insert_with(TextSystem::new);
    text.set_size(appearance.text_size);
    text
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
    pub(crate) closed_by_button: bool,
    text: Option<TextSystem>,
    shown_frame: Option<Frame>,
    planned: Option<FrameTime>,
    shown_opaque: Option<Rect>,
    pub(crate) app_cursor: Cursor,
    pub(crate) accessibility: Accessibility,
    pub(crate) appearance: Rc<RefCell<AppearanceWatch>>,
}

fn layer_of(kind: LayerKind) -> zwlr_layer_shell_v1::Layer {
    match kind {
        LayerKind::Background => zwlr_layer_shell_v1::Layer::Background,
        LayerKind::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
        LayerKind::Top => zwlr_layer_shell_v1::Layer::Top,
        LayerKind::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
    }
}

fn anchor_of(edges: Edges) -> zwlr_layer_surface_v1::Anchor {
    let mut anchor = zwlr_layer_surface_v1::Anchor::empty();
    for (on, edge) in [
        (edges.top, zwlr_layer_surface_v1::Anchor::Top),
        (edges.bottom, zwlr_layer_surface_v1::Anchor::Bottom),
        (edges.left, zwlr_layer_surface_v1::Anchor::Left),
        (edges.right, zwlr_layer_surface_v1::Anchor::Right),
    ] {
        if on {
            anchor |= edge;
        }
    }
    anchor
}

fn keyboard_interactivity(focus: KeyboardFocus) -> zwlr_layer_surface_v1::KeyboardInteractivity {
    match focus {
        KeyboardFocus::None => zwlr_layer_surface_v1::KeyboardInteractivity::None,
        KeyboardFocus::OnDemand => zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
        KeyboardFocus::Exclusive => zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
    }
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

    fn on(
        connection: Connection,
        role: Role,
        size: (u32, u32),
        shared_appearance: Option<Rc<RefCell<AppearanceWatch>>>,
        output: Option<String>,
    ) -> Result<Self, WindowError> {
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
        let role = match role {
            Role::Toplevel { title, app_id } => {
                let wm_base = state
                    .wm_base
                    .clone()
                    .ok_or(WindowError::Missing("xdg_wm_base"))?;
                let xdg_surface = wm_base.get_xdg_surface(&surface, &handle, ());
                let toplevel = xdg_surface.get_toplevel(&handle, ());
                toplevel.set_title(title.clone());
                toplevel.set_app_id(app_id);
                chrome = Some(Chrome::new(title));
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

        let (extra_width, extra_height) = if chrome.is_some() {
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
            closed_by_button: false,
            text: None,
            shown_frame: None,
            planned: None,
            shown_opaque: None,
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

    pub fn header_height(&self) -> u32 {
        self.title_height()
    }

    pub(crate) fn content_offset(&self) -> Point {
        let frame = self.frame();
        let origin = frame.content_origin();
        Point::new(origin.x - frame.margin, origin.y - frame.margin)
    }

    fn geometry(&self) -> (u32, u32) {
        self.state.suggested_size.unwrap_or(self.size)
    }

    fn title_height(&self) -> u32 {
        match &self.chrome {
            Some(_) if !self.state.toplevel.fullscreen => WINDOW_HEADER_HEIGHT,
            _ => 0,
        }
    }

    fn resize_margin(&self) -> u32 {
        let toplevel = self.state.toplevel;
        let floating = !(toplevel.maximized || toplevel.fullscreen || toplevel.tiled);
        let transparent = match &self.gpu {
            Gpu::Ready(gpu) => gpu.transparent,
            Gpu::Starting(_) | Gpu::Released | Gpu::Unavailable => true,
        };
        if self.chrome.is_some() && floating && transparent {
            RESIZE_MARGIN
        } else {
            0
        }
    }

    fn frame(&self) -> Frame {
        let (width, height) = self.geometry();
        let title = self.title_height();
        Frame {
            margin: self.resize_margin() as f32,
            title: title as f32,
            inset: if title > 0 { SHEET_INSET as f32 } else { 0.0 },
            regions: self.chrome.as_ref().and_then(|chrome| chrome.regions),
            header: self
                .chrome
                .as_ref()
                .is_some_and(|chrome| chrome.header.is_some()),
            width: width as f32,
            height: height as f32,
        }
    }

    pub fn set_title(&mut self, title: &str) {
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

    pub fn close_requested_by_button(&self) -> bool {
        self.closed_by_button
    }

    fn toggle_full_screen(&mut self) -> Result<(), WindowError> {
        let toplevel = self.toplevel()?;
        if self.state.toplevel.fullscreen {
            toplevel.unset_fullscreen();
        } else {
            toplevel.set_fullscreen(None);
        }
        self.flush()
    }

    fn move_and_resize_items(&self) -> Vec<MenuItem> {
        let toplevel = self.state.toplevel;
        let place = |key: &str| MenuItem::new(&L10N.tr(key), key);
        let mut items = vec![
            place("place-left").shortcut("Ctrl+Super+Left"),
            place("place-right").shortcut("Ctrl+Super+Right"),
            MenuItem::separator(),
            place("place-top-left"),
            place("place-top-right"),
            place("place-bottom-left"),
            place("place-bottom-right"),
            MenuItem::separator(),
            place("place-fill").enabled(!toplevel.maximized),
        ];
        if self.can_place() && self.state.outputs.len() > 1 {
            items.push(place("place-next-display").shortcut("Alt+Shift+Super+Right"));
        }
        items.extend([
            MenuItem::separator(),
            place("place-restore").enabled(toplevel.tiled || toplevel.maximized),
        ]);
        items
    }

    fn offers_tiling_menu(&self) -> bool {
        self.can_place() && !self.state.toplevel.fullscreen && self.menus.is_none()
    }

    fn open_tiling_menu(&mut self) -> Result<(), WindowError> {
        let control = Control::Zoom.area(self.frame().title);
        let offset = self.content_offset();
        self.show_menu(
            Menu::new(self.move_and_resize_items()),
            MenuPlacement::Below(Rect::new(
                control.x - offset.x,
                control.y - offset.y,
                control.width,
                control.height,
            )),
        )?;
        self.window_menu = true;
        Ok(())
    }

    fn open_window_menu(&mut self, at: Point) -> Result<(), WindowError> {
        let toplevel = self.state.toplevel;
        let mut items = vec![
            MenuItem::new(&L10N.tr("minimize"), "minimize")
                .enabled(!toplevel.fullscreen)
                .shortcut("Super+M"),
            MenuItem::new(
                &L10N.tr(if toplevel.maximized { "unzoom" } else { "zoom" }),
                "zoom",
            )
            .enabled(!toplevel.fullscreen),
            MenuItem::new(
                &L10N.tr(if toplevel.fullscreen {
                    "exit-full-screen"
                } else {
                    "enter-full-screen"
                }),
                "full-screen",
            )
            .shortcut("Ctrl+Super+F"),
            MenuItem::separator(),
        ];
        if self.can_place() {
            items.push(
                MenuItem::submenu(
                    &L10N.tr("move-and-resize"),
                    Menu::new(self.move_and_resize_items()),
                )
                .enabled(!toplevel.fullscreen),
            );
        }
        if self.can_keep_on_top() {
            items.push(
                MenuItem::new(&L10N.tr("keep-on-top"), "keep-on-top")
                    .checked(self.keep_on_top)
                    .enabled(!toplevel.fullscreen),
            );
        }
        items.extend([
            MenuItem::new(&L10N.tr("move"), "move").enabled(!toplevel.fullscreen),
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("close"), "close").shortcut("Super+W"),
            MenuItem::new(&L10N.tr("quit-app"), "quit").shortcut("Super+Q"),
        ]);
        let offset = self.content_offset();
        self.show_menu(
            Menu::new(items),
            MenuPlacement::AtPoint(Point::new(at.x - offset.x, at.y - offset.y)),
        )?;
        self.window_menu = true;
        Ok(())
    }

    fn window_menu_action(&mut self, action: &str) -> Result<(), WindowError> {
        use unbound_protocols::client::unbound_app_v1::Placement;
        match action {
            "minimize" => self.minimize(),
            "zoom" => self.toggle_zoom(),
            "full-screen" => self.toggle_full_screen(),
            "move" => self.start_move(),
            "close" => {
                self.state.events.push(WindowEvent::CloseRequested);
                Ok(())
            }
            "quit" => {
                self.state.events.push(WindowEvent::QuitRequested);
                Ok(())
            }
            "place-left" => self.place(Placement::Left),
            "place-right" => self.place(Placement::Right),
            "place-top-left" => self.place(Placement::TopLeft),
            "place-top-right" => self.place(Placement::TopRight),
            "place-bottom-left" => self.place(Placement::BottomLeft),
            "place-bottom-right" => self.place(Placement::BottomRight),
            "place-fill" if !self.state.toplevel.maximized => self.toggle_zoom(),
            "place-next-display" => self.place(Placement::NextDisplay),
            "place-restore" => self.place(Placement::Restore),
            "keep-on-top" => {
                let keep = !self.keep_on_top;
                self.set_keep_on_top(keep)?;
                self.keep_on_top = keep;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn toplevel(&self) -> Result<&XdgToplevel, WindowError> {
        match &self.role {
            ShellSurface::Toplevel(_, toplevel) => Ok(toplevel),
            _ => Err(WindowError::Missing("a toplevel surface")),
        }
    }

    fn grab(&self) -> Result<(&WlSeat, u32), WindowError> {
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

    fn start_move(&mut self) -> Result<(), WindowError> {
        let (seat, serial) = self.grab()?;
        self.toplevel()?._move(seat, serial);
        self.flush()
    }

    fn start_resize(&mut self, edge: ResizeEdge) -> Result<(), WindowError> {
        let (seat, serial) = self.grab()?;
        self.toplevel()?.resize(seat, serial, resize_protocol(edge));
        self.flush()
    }

    pub fn minimize(&mut self) -> Result<(), WindowError> {
        self.toplevel()?.set_minimized();
        self.flush()
    }

    pub fn toggle_zoom(&mut self) -> Result<(), WindowError> {
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
        Ok(())
    }

    pub(crate) fn has_events(&self) -> bool {
        !self.state.events.is_empty()
    }

    pub(crate) fn dispatch_pending(&mut self) -> Result<(), WindowError> {
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }

    pub(crate) fn flush(&mut self) -> Result<(), WindowError> {
        match self.connection.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn take_events(&mut self) -> Vec<WindowEvent> {
        self.take_arrival();
        self.take_drop();
        self.serve_menus(self.frame_time());
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

    pub fn show_menu(&mut self, menu: Menu, placement: MenuPlacement) -> Result<(), WindowError> {
        self.close_menu();
        let offset = self.content_offset();
        let placement = match placement {
            MenuPlacement::AtPoint(point) => {
                MenuPlacement::AtPoint(Point::new(point.x + offset.x, point.y + offset.y))
            }
            MenuPlacement::Below(rect) => MenuPlacement::Below(Rect::new(
                rect.x + offset.x,
                rect.y + offset.y,
                rect.width,
                rect.height,
            )),
        };
        let parent = match &self.role {
            ShellSurface::Toplevel(xdg_surface, _) => Parent::Toplevel(xdg_surface.clone()),
            ShellSurface::Layer(layer) => Parent::Layer(layer.clone()),
            ShellSurface::Lock(_) => {
                return Err(WindowError::Missing("a surface that takes popups"));
            }
        };
        let frame = self.frame_time();
        let opacity = self.fading.take().map_or(0.0, |fading| {
            let opacity = fading.opacity(frame.on_screen());
            fading.destroy();
            opacity
        });
        let handle = self.queue.handle();
        let appearance = self.appearance();
        let opening = Opening {
            state: &mut self.state,
            handle: &handle,
            text: menu_text(&mut self.text, appearance),
            palette: appearance.palette(),
        };
        let menus = Menus::open(opening, menu, parent, placement, opacity)?;
        self.menus = Some(menus);
        self.flush()?;
        while self.menus.as_ref().is_some_and(Menus::waiting) {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        self.draw_menus(self.frame_time())
    }

    pub fn drive(
        &mut self,
        ui: &mut Ui,
        event: WindowEvent,
    ) -> Result<Option<WindowEvent>, WindowError> {
        let (changed, unhandled) = match event {
            WindowEvent::Input(input) => {
                let changed = ui.handle(&input);
                if let Some((menu, at)) = ui.take_menu_request() {
                    self.show_menu(menu, MenuPlacement::AtPoint(at))?;
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
                (changed, None)
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
            WindowEvent::CapsLock(on) => {
                ui.context().caps_lock = on;
                (true, Some(WindowEvent::CapsLock(on)))
            }
            event => (false, Some(event)),
        };
        if changed || ui.is_animating() {
            self.request_redraw();
        }
        self.ui_deadline = ui.deadline();
        Ok(unhandled)
    }

    pub fn is_menu_open(&self) -> bool {
        self.menus.is_some()
    }

    pub fn close_menu(&mut self) {
        self.window_menu = false;
        if let Some(menus) = self.menus.take() {
            if let Some(fading) = self.fading.take() {
                fading.destroy();
            }
            let handle = self.queue.handle();
            let now = self.frame_time().on_screen();
            self.fading = Some(menus.close(&mut self.state, &handle, now));
            self.state.popup_input.clear();
            self.accessibility.request_tree();
            let _ = self.flush();
        }
    }

    fn draw_fading(&mut self, frame: FrameTime) {
        let handle = self.queue.handle();
        let Some(fading) = &mut self.fading else {
            return;
        };
        match fading.draw(&self.state, &handle, frame) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                if let Some(fading) = self.fading.take() {
                    fading.destroy();
                }
            }
        }
        let _ = self.flush();
    }

    fn draw_menus(&mut self, now: FrameTime) -> Result<(), WindowError> {
        let appearance = self.appearance.borrow().current();
        let scale = self.scale();
        let handle = self.queue.handle();
        if let Some(menus) = &mut self.menus {
            menus.draw(
                &mut self.state,
                &handle,
                &mut self.cpu,
                appearance,
                scale,
                now,
            )?;
        }
        self.accessibility.request_tree();
        self.flush()
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
            .chain(self.tiling_menu_deadline())
            .min()
    }

    fn tiling_menu_deadline(&self) -> Option<Instant> {
        self.chrome
            .as_ref()
            .and_then(|chrome| chrome.zoom_rest)
            .map(|since| since + TILING_MENU_AFTER)
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
            .tiling_menu_deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            let offered = self.offers_tiling_menu();
            if let Some(chrome) = &mut self.chrome {
                chrome.zoom_rest = None;
                if offered {
                    chrome.pressed = None;
                    self.needs_redraw = true;
                }
            }
            if offered && self.open_tiling_menu().is_err() {
                self.needs_redraw = true;
            }
        }
        if self.ui_deadline.is_some_and(|deadline| deadline <= now) {
            self.ui_deadline = None;
            self.request_redraw();
        }
    }

    fn serve_menus(&mut self, frame: FrameTime) {
        let now = frame.on_screen();
        let changed = std::mem::take(&mut self.state.popups_changed);
        if changed && self.fading.is_some() {
            self.draw_fading(frame);
        }
        if self.menus.is_none() {
            self.state.popup_input.clear();
            return;
        }
        let mut outcomes = Vec::new();
        if self.menus.as_ref().is_some_and(Menus::dismissed) {
            outcomes.push((MenuOutcome::Dismiss, false));
        }
        let input = std::mem::take(&mut self.state.popup_input);
        if let Some(menus) = &mut self.menus {
            for (level, event) in input {
                let keyboard = matches!(event, Event::Key { .. });
                if keyboard && menus.tracker().is_flashing() {
                    self.state.events.push(WindowEvent::Input(event));
                    continue;
                }
                outcomes.push((menus.handle(level, &event, now), keyboard));
            }
            outcomes.push((menus.tick(Instant::now().max(now)), false));
        }
        let mut redraw = changed
            || self
                .menus
                .as_ref()
                .is_some_and(|menus| menus.tracker().is_flashing());
        for (outcome, keyboard) in outcomes {
            redraw |= self.apply_menu_outcome(outcome, keyboard);
        }
        if redraw
            && self.menus.is_some()
            && let Err(error) = self.draw_menus(frame)
        {
            tracing::warn!(%error, "cannot draw the menus");
        }
    }

    fn apply_menu_outcome(&mut self, outcome: MenuOutcome, keyboard: bool) -> bool {
        match outcome {
            MenuOutcome::Nothing => false,
            MenuOutcome::Redraw => true,
            MenuOutcome::Activate(action) if self.window_menu => {
                if let Err(error) = self.window_menu_action(&action) {
                    tracing::warn!(%error, %action, "cannot perform the window menu action");
                }
                true
            }
            MenuOutcome::Activate(action) => {
                self.state.events.push(WindowEvent::MenuAction(action));
                true
            }
            MenuOutcome::Dismiss => {
                let activated = self
                    .menus
                    .as_ref()
                    .is_some_and(|menus| menus.tracker().is_flashing());
                let window_menu = self.window_menu;
                self.close_menu();
                if !activated && !window_menu {
                    self.state.events.push(WindowEvent::MenuClosed);
                }
                false
            }
            MenuOutcome::CloseFrom(level) => {
                if let Some(menus) = &mut self.menus {
                    menus.close_from(&mut self.state, level);
                }
                true
            }
            MenuOutcome::Search(menu) => {
                let handle = self.queue.handle();
                let appearance = self.appearance();
                if let Some(menus) = &mut self.menus {
                    let opening = Opening {
                        state: &mut self.state,
                        handle: &handle,
                        text: menu_text(&mut self.text, appearance),
                        palette: appearance.palette(),
                    };
                    if let Err(error) = menus.search(opening, menu) {
                        tracing::warn!(%error, "cannot resize the searched menu");
                    }
                }
                true
            }
            MenuOutcome::OpenSubmenu { level, index } => {
                let handle = self.queue.handle();
                let appearance = self.appearance();
                if let Some(menus) = &mut self.menus {
                    let opening = Opening {
                        state: &mut self.state,
                        handle: &handle,
                        text: menu_text(&mut self.text, appearance),
                        palette: appearance.palette(),
                    };
                    if let Err(error) = menus.open_submenu(opening, level, index, keyboard) {
                        tracing::warn!(%error, "cannot open the submenu");
                    }
                }
                true
            }
        }
    }

    pub(crate) fn pointer_in_content(&self) -> bool {
        self.chrome
            .as_ref()
            .is_none_or(|chrome| chrome.pointer_in_content)
    }

    fn show_cursor(&mut self, cursor: Cursor) {
        self.state.input.set_cursor(cursor);
    }

    fn through_chrome(&mut self, event: WindowEvent, passed: &mut Vec<WindowEvent>) {
        let frame = self.frame();
        let origin = frame.content_origin();
        let into_content = |pointer: Pointer| Pointer {
            position: Point::new(pointer.position.x - origin.x, pointer.position.y - origin.y),
            ..pointer
        };
        let now = self.frame_time().on_screen();
        let reduce_motion = self.appearance.borrow().current().reduce_motion;
        let Some(chrome) = self.chrome.as_mut() else {
            passed.push(event);
            return;
        };
        let input = match event {
            WindowEvent::Input(input) => input,
            WindowEvent::Drag(drag) => {
                let shift = |point: Point| Point::new(point.x - origin.x, point.y - origin.y);
                passed.push(WindowEvent::Drag(match drag {
                    DragEvent::Entered {
                        position,
                        uris,
                        text,
                    } => DragEvent::Entered {
                        position: shift(position),
                        uris,
                        text,
                    },
                    DragEvent::Moved(position) => DragEvent::Moved(shift(position)),
                    DragEvent::Dropped {
                        position,
                        transfer,
                        action,
                    } => DragEvent::Dropped {
                        position: shift(position),
                        transfer,
                        action,
                    },
                    other => other,
                }));
                return;
            }
            WindowEvent::ToplevelChanged(_) => {
                self.needs_redraw = true;
                passed.push(event);
                return;
            }
            other => {
                passed.push(other);
                return;
            }
        };
        match input {
            Event::PointerEnter(pointer) | Event::PointerMove(pointer) => {
                if let Some(start) = chrome.tab_press
                    && (pointer.position.x - start.x).hypot(pointer.position.y - start.y)
                        > CLICK_SLOP
                {
                    chrome.tab_press = None;
                    chrome.content_buttons = 0;
                    if std::mem::take(&mut chrome.pointer_in_content) {
                        passed.push(WindowEvent::Input(Event::PointerLeave));
                    }
                    if self.start_move().is_err() {
                        self.needs_redraw = true;
                    }
                    return;
                }
                let zone = chrome.zone_at(&frame, pointer.position);
                if let Some(control) = chrome.pressed {
                    let inside = zone == Zone::Control(control);
                    if inside != chrome.pressed_inside {
                        chrome.pressed_inside = inside;
                        chrome.zoom_rest = None;
                        self.needs_redraw = true;
                    }
                    return;
                }
                if chrome.hover(&frame, Some(pointer.position), now, reduce_motion) {
                    self.needs_redraw = true;
                }
                let content = zone == Zone::Content || chrome.content_buttons > 0;
                if content {
                    let entering = !chrome.pointer_in_content;
                    chrome.pointer_in_content = true;
                    let pointer = into_content(pointer);
                    passed.push(WindowEvent::Input(if entering {
                        Event::PointerEnter(pointer)
                    } else {
                        Event::PointerMove(pointer)
                    }));
                } else if chrome.pointer_in_content {
                    chrome.pointer_in_content = false;
                    passed.push(WindowEvent::Input(Event::PointerLeave));
                }
                let cursor = match zone {
                    _ if content => self.app_cursor,
                    Zone::Resize(edge) => resize_cursor(edge),
                    _ => Cursor::Default,
                };
                self.show_cursor(cursor);
            }
            Event::PointerDown(pointer) => {
                let zone = chrome.zone_at(&frame, pointer.position);
                if zone == Zone::Content || chrome.content_buttons > 0 {
                    if chrome.content_buttons == 0
                        && pointer.button == PointerButton::Primary
                        && chrome.tab_at(&frame, pointer.position)
                    {
                        chrome.tab_press = Some(pointer.position);
                    }
                    chrome.content_buttons += 1;
                    passed.push(WindowEvent::Input(Event::PointerDown(into_content(
                        pointer,
                    ))));
                    return;
                }
                if pointer.button == PointerButton::Secondary
                    && matches!(zone, Zone::TitleBar | Zone::Control(_))
                {
                    if self.open_window_menu(pointer.position).is_err() {
                        self.needs_redraw = true;
                    }
                    return;
                }
                if pointer.button != PointerButton::Primary {
                    return;
                }
                let result = match zone {
                    Zone::Control(control) => {
                        chrome.pressed = Some(control);
                        chrome.pressed_inside = true;
                        chrome.zoom_rest = (control == Control::Zoom).then_some(now);
                        self.needs_redraw = true;
                        Ok(())
                    }
                    Zone::TitleBar if pointer.count % 2 == 0 => self.toggle_zoom(),
                    Zone::TitleBar => self.start_move(),
                    Zone::Resize(edge) => self.start_resize(edge),
                    Zone::Content | Zone::Outside => Ok(()),
                };
                if result.is_err() {
                    self.needs_redraw = true;
                }
            }
            Event::PointerUp(pointer) => {
                chrome.tab_press = None;
                chrome.zoom_rest = None;
                if chrome.content_buttons > 0 {
                    chrome.content_buttons -= 1;
                    passed.push(WindowEvent::Input(Event::PointerUp(into_content(pointer))));
                    return;
                }
                let Some(control) = chrome.pressed.take() else {
                    return;
                };
                self.needs_redraw = true;
                if chrome.zone_at(&frame, pointer.position) != Zone::Control(control) {
                    return;
                }
                let result = match control {
                    Control::Close => {
                        self.closed_by_button = true;
                        passed.push(WindowEvent::CloseRequested);
                        Ok(())
                    }
                    Control::Minimize => self.minimize(),
                    Control::Zoom if pointer.modifiers.alt => self.toggle_full_screen(),
                    Control::Zoom => self.toggle_zoom(),
                };
                if result.is_err() {
                    self.needs_redraw = true;
                }
            }
            Event::ModifiersChanged(modifiers) => {
                if chrome.alternate != modifiers.alt {
                    chrome.alternate = modifiers.alt;
                    self.needs_redraw = true;
                }
                passed.push(WindowEvent::Input(input));
            }
            Event::PointerLeave => {
                chrome.content_buttons = 0;
                chrome.tab_press = None;
                if chrome.hover(&frame, None, now, reduce_motion) {
                    self.needs_redraw = true;
                }
                if std::mem::take(&mut chrome.pointer_in_content) {
                    passed.push(WindowEvent::Input(Event::PointerLeave));
                }
            }
            Event::Scroll(scroll) => {
                if chrome.zone_at(&frame, scroll.position) == Zone::Content {
                    passed.push(WindowEvent::Input(Event::Scroll(
                        unbound_graphics::Scroll {
                            position: Point::new(
                                scroll.position.x - origin.x,
                                scroll.position.y - origin.y,
                            ),
                            ..scroll
                        },
                    )));
                }
            }
            other => passed.push(WindowEvent::Input(other)),
        }
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

    fn perform_chrome(&mut self, request: &ActionRequest) -> bool {
        let Some(control) = control_for_node(request.target_node) else {
            return false;
        };
        if self.chrome.is_none() || request.action != accesskit::Action::Click {
            return false;
        }
        let result = match control {
            Control::Close => {
                self.request_close();
                Ok(())
            }
            Control::Minimize => self.minimize(),
            Control::Zoom => self.toggle_zoom(),
        };
        result.is_ok()
    }

    fn framed(&mut self, frame: &Frame, scene: Scene, presentation: Instant) -> Scene {
        let appearance = self.appearance.borrow().current();
        let toplevel = self.state.toplevel;
        let look = Look {
            active: toplevel.activated,
            zoomed: toplevel.maximized || toplevel.fullscreen,
        };
        match &mut self.chrome {
            Some(chrome) => chrome.frame_scene(
                frame,
                scene,
                &appearance.palette(),
                appearance.scheme,
                look,
                presentation,
            ),
            None => scene,
        }
    }

    fn declare_opaque(&mut self, opaque: Option<Rect>) {
        if self.shown_opaque == opaque {
            return;
        }
        let Some(compositor) = &self.state.compositor else {
            return;
        };
        self.shown_opaque = opaque;
        let Some(area) = opaque else {
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
        self.surface.set_opaque_region(Some(&region));
        region.destroy();
    }

    fn frame_surface(&mut self, frame: &Frame) {
        if self.shown_frame.as_ref() == Some(frame) {
            return;
        }
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

    pub fn set_exclusive_zone(&mut self, zone: i32) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_exclusive_zone(zone);
            self.needs_redraw = true;
        }
    }

    pub fn set_layer(&mut self, kind: LayerKind) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_layer(layer_of(kind));
            self.needs_redraw = true;
        }
    }

    pub fn set_layer_anchor(&mut self, anchor: Edges) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_anchor(anchor_of(anchor));
            self.needs_redraw = true;
        }
    }

    pub fn set_layer_margin(&mut self, top: i32) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_margin(top, 0, 0, 0);
            self.needs_redraw = true;
        }
    }

    pub fn set_layer_size(&mut self, size: (u32, u32)) -> Result<(), WindowError> {
        let ShellSurface::Layer(layer) = &self.role else {
            return Err(WindowError::Missing("a layer surface"));
        };
        layer.set_size(size.0, size.1);
        if !self.visible {
            return Ok(());
        }
        self.state.configured = false;
        self.surface.commit();
        while !self.state.configured {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        self.needs_redraw = true;
        Ok(())
    }

    pub fn set_input_region(&mut self, region: Option<&[Rect]>) {
        let Some(rects) = region else {
            self.surface.set_input_region(None);
            self.needs_redraw = true;
            return;
        };
        let Some(compositor) = &self.state.compositor else {
            return;
        };
        let wayland_region = compositor.create_region(&self.queue.handle(), ());
        let offset = self.content_offset();
        for rect in rects {
            let left = (rect.x + offset.x).floor() as i32;
            let top = (rect.y + offset.y).floor() as i32;
            wayland_region.add(
                left,
                top,
                (rect.x + offset.x + rect.width).ceil() as i32 - left,
                (rect.y + offset.y + rect.height).ceil() as i32 - top,
            );
        }
        self.surface.set_input_region(Some(&wayland_region));
        wayland_region.destroy();
        self.needs_redraw = true;
    }

    pub fn set_keyboard_focus(&mut self, focus: KeyboardFocus) -> Result<(), WindowError> {
        let ShellSurface::Layer(layer) = &self.role else {
            return Err(WindowError::Missing("a layer surface"));
        };
        layer.set_keyboard_interactivity(keyboard_interactivity(focus));
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
        self.size = (scene.width + extra_width, scene.height + extra_height);
        let (scene, opaque) = if self.chrome.is_some() {
            let frame = self.frame();
            let sheet_opaque = scene.background.a == u8::MAX;
            let framed = self.framed(&frame, scene, frame_time.presentation);
            self.frame_surface(&frame);
            let chassis = self.appearance.borrow().current().palette().chassis;
            let opaque = (chassis.a == u8::MAX && sheet_opaque).then(|| frame.window());
            (framed, opaque)
        } else {
            let opaque = (scene.background.a == u8::MAX)
                .then(|| Rect::new(0.0, 0.0, scene.width as f32, scene.height as f32));
            (scene, opaque)
        };
        self.declare_opaque(opaque);
        self.needs_redraw = self.chrome.as_ref().is_some_and(|chrome| chrome.animating);
        let handle = self.queue.handle();
        self.surface.frame(&handle, ());
        self.state
            .presentation
            .committing(&self.surface, &handle, frame_time);
        self.state.frame_pending = true;
        let scale = self.scale();
        self.drawn_scale = scale;
        if let Some(surface_scale) = &self.surface_scale {
            surface_scale.show_at((scene.width, scene.height));
        }
        let damage = self
            .shown_scene
            .take()
            .filter(|(_, shown_scale)| *shown_scale == scale)
            .and_then(|(shown, _)| damage(&shown, &scene, scale));
        let presented = match &mut self.gpu {
            Gpu::Ready(gpu) => Some(gpu.present(&scene, scale, damage.as_deref())),
            Gpu::Starting(_) | Gpu::Released | Gpu::Unavailable => None,
        };
        let path = match presented {
            Some(Ok(())) => {
                if self.shm_target.is_holding() {
                    self.shm_target.destroy();
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
        self.flush()?;
        Ok(path)
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
        self.shm_target.present(
            &mut self.cpu,
            shm,
            &self.queue.handle(),
            &self.surface,
            scene,
            self.drawn_scale,
            damage,
        )
    }
}

#[derive(Default)]
pub(crate) struct ShmTarget {
    buffers: Vec<ShmBuffer>,
    bgra: Vec<u8>,
    scratch: Vec<u8>,
    size: (u32, u32),
}

fn copy_swizzled(source: &[u8], target: &mut [u8]) {
    for (to, from) in target
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(source.as_chunks::<4>().0)
    {
        *to = [from[2], from[1], from[0], from[3]];
    }
}

impl ShmTarget {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn present(
        &mut self,
        cpu: &mut CpuRenderer,
        shm: &WlShm,
        handle: &QueueHandle<State>,
        surface: &WlSurface,
        scene: &Scene,
        scale: f64,
        damage: Option<&[Region]>,
    ) -> Result<(), WindowError> {
        let damage = self.render(cpu, scene, scale, damage)?;
        self.commit(shm, handle, surface, damage, false)
    }

    pub(crate) fn render<'a>(
        &mut self,
        cpu: &mut CpuRenderer,
        scene: &Scene,
        scale: f64,
        damage: Option<&'a [Region]>,
    ) -> Result<Option<&'a [Region]>, WindowError> {
        let (width, height) = (pixels(scene.width, scale), pixels(scene.height, scale));
        let bytes = width as usize * height as usize * 4;
        let damage = damage.filter(|_| self.bgra.len() == bytes);
        match damage {
            Some(regions) => self.render_regions(cpu, scene, scale, width, regions)?,
            None => {
                self.bgra.resize(bytes, 0);
                cpu.render_into(scene, scale, &mut self.bgra)?;
                for pixel in self.bgra.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
            }
        }
        self.size = (width, height);
        Ok(damage)
    }

    pub(crate) fn present_moved(
        &mut self,
        shm: &WlShm,
        handle: &QueueHandle<State>,
        surface: &WlSurface,
        alpha: u32,
        (dx, dy): (i32, i32),
    ) -> Result<(), WindowError> {
        if alpha >= 256 && (dx, dy) == (0, 0) {
            return self.commit(shm, handle, surface, None, false);
        }
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let stride = width * 4;
        let columns = width.saturating_sub(dx.unsigned_abs() as usize) * 4;
        let (from_x, to_x) = match dx >= 0 {
            true => (0, dx as usize * 4),
            false => (dx.unsigned_abs() as usize * 4, 0),
        };
        self.scratch.clear();
        self.scratch.resize(self.bgra.len(), 0);
        for (row, target) in self.scratch.chunks_exact_mut(stride).enumerate() {
            let Some(source) = row
                .checked_add_signed(-(dy as isize))
                .filter(|source| *source < height)
                .and_then(|source| self.bgra.chunks_exact(stride).nth(source))
            else {
                continue;
            };
            for (to, from) in target[to_x..to_x + columns]
                .iter_mut()
                .zip(&source[from_x..from_x + columns])
            {
                *to = ((u32::from(*from) * alpha) >> 8) as u8;
            }
        }
        self.commit(shm, handle, surface, None, true)
    }

    fn commit(
        &mut self,
        shm: &WlShm,
        handle: &QueueHandle<State>,
        surface: &WlSurface,
        damage: Option<&[Region]>,
        faded: bool,
    ) -> Result<(), WindowError> {
        let (width, height) = self.size;
        self.buffers.retain(|buffer| {
            buffer.width == width && buffer.height == height || buffer.busy.load(Ordering::Acquire)
        });
        for buffer in &mut self.buffers {
            buffer.stale = match (buffer.stale.take(), damage) {
                (Some(mut stale), Some(regions))
                    if stale.len() + regions.len() <= MAX_STALE_REGIONS =>
                {
                    stale.extend_from_slice(regions);
                    Some(stale)
                }
                _ => None,
            };
        }
        let index = match self.buffers.iter().position(|buffer| {
            buffer.width == width && buffer.height == height && !buffer.busy.load(Ordering::Acquire)
        }) {
            Some(index) => index,
            None => {
                self.buffers
                    .push(create_buffer(shm, handle, width, height)?);
                self.buffers.len() - 1
            }
        };
        let buffer = &mut self.buffers[index];
        let pixels = match faded {
            true => &self.scratch,
            false => &self.bgra,
        };
        match buffer.stale.replace(Vec::new()) {
            None => buffer.file.write_all_at(pixels, 0)?,
            Some(stale) => {
                let stride = width as usize * 4;
                for region in stale {
                    for row in region.y..region.y + region.height {
                        let start = row as usize * stride + region.x as usize * 4;
                        let end = start + region.width as usize * 4;
                        buffer
                            .file
                            .write_all_at(&pixels[start..end], start as u64)?;
                    }
                }
            }
        }
        buffer.busy.store(true, Ordering::Release);
        surface.attach(Some(&buffer.buffer), 0, 0);
        match damage {
            Some(regions) => {
                for region in regions {
                    surface.damage_buffer(
                        clamp_i32(region.x),
                        clamp_i32(region.y),
                        clamp_i32(region.width),
                        clamp_i32(region.height),
                    );
                }
            }
            None => surface.damage_buffer(0, 0, clamp_i32(width), clamp_i32(height)),
        }
        surface.commit();
        Ok(())
    }

    fn render_regions(
        &mut self,
        cpu: &mut CpuRenderer,
        scene: &Scene,
        scale: f64,
        width: u32,
        regions: &[Region],
    ) -> Result<(), WindowError> {
        let stride = width as usize * 4;
        for region in regions {
            let row_bytes = region.width as usize * 4;
            self.scratch.clear();
            self.scratch.resize(row_bytes * region.height as usize, 0);
            cpu.render_region(scene, scale, *region, &mut self.scratch)?;
            for (row, source) in self.scratch.chunks_exact(row_bytes).enumerate() {
                let start = (region.y as usize + row) * stride + region.x as usize * 4;
                copy_swizzled(source, &mut self.bgra[start..start + row_bytes]);
            }
        }
        Ok(())
    }

    fn is_holding(&self) -> bool {
        !self.buffers.is_empty() || self.bgra.capacity() > 0
    }

    pub(crate) fn destroy(&mut self) {
        for buffer in self.buffers.drain(..) {
            buffer.buffer.destroy();
        }
        self.bgra = Vec::new();
        self.scratch = Vec::new();
    }
}

fn create_buffer(
    shm: &WlShm,
    handle: &QueueHandle<State>,
    width: u32,
    height: u32,
) -> Result<ShmBuffer, WindowError> {
    let stride = clamp_i32(width.saturating_mul(4));
    let length = u64::from(width) * u64::from(height) * 4;
    let file =
        File::from(memfd_create("unbound-toolkit", MemfdFlags::CLOEXEC).map_err(io::Error::from)?);
    file.set_len(length)?;
    let pool = shm.create_pool(
        file.as_fd(),
        clamp_i32(u32::try_from(length).unwrap_or(u32::MAX)),
        handle,
        (),
    );
    let busy = Arc::new(AtomicBool::new(false));
    let buffer = pool.create_buffer(
        0,
        clamp_i32(width),
        clamp_i32(height),
        stride,
        wl_shm::Format::Argb8888,
        handle,
        busy.clone(),
    );
    pool.destroy();
    Ok(ShmBuffer {
        buffer,
        file,
        width,
        height,
        busy,
        stale: None,
    })
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
        let _ = self.connection.flush();
    }
}

impl GpuSurface {
    fn present(
        &mut self,
        scene: &Scene,
        scale: f64,
        damage: Option<&[Region]>,
    ) -> Result<(), DrawError> {
        let size = (pixels(scene.width, scale), pixels(scene.height, scale));
        if self.configured != Some(size) {
            self.config.width = size.0;
            self.config.height = size.1;
            self.gpu.configure(&self.surface, &self.config)?;
            self.configured = Some(size);
        }
        self.renderer.present(scene, scale, &self.surface, damage)
    }
}

fn clamp_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererChoice {
    Automatic,
    Gpu,
    Cpu,
}

fn renderer_choice() -> RendererChoice {
    match std::env::var(RENDERER_VARIABLE).as_deref() {
        Ok("gpu") => RendererChoice::Gpu,
        Ok("cpu") => RendererChoice::Cpu,
        _ => RendererChoice::Automatic,
    }
}

fn start_gpu(connection: &Connection, surface: &WlSurface, atlas: (u32, u32)) -> Gpu {
    let choice = renderer_choice();
    if choice == RendererChoice::Cpu
        || choice == RendererChoice::Automatic && GpuDevice::known_software_only()
    {
        return Gpu::Unavailable;
    }
    let allow_software = choice == RendererChoice::Gpu;
    let display = connection.backend().display_ptr() as usize;
    let wl_surface = surface.id().as_ptr() as usize;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let opened = open_gpu(display, wl_surface, atlas, allow_software);
        if let Err(error @ DrawError::Software(_)) = &opened {
            tracing::info!("drawing on the CPU: {error}");
        }
        let _ = sender.send(opened);
    });
    Gpu::Starting(receiver)
}

fn open_gpu(
    display: usize,
    surface: usize,
    atlas: (u32, u32),
    allow_software: bool,
) -> Result<GpuSurface, DrawError> {
    let missing = || DrawError::Gpu("the Wayland surface has no libwayland pointer".to_owned());
    let display = NonNull::new(display as *mut c_void).ok_or_else(missing)?;
    let window = NonNull::new(surface as *mut c_void).ok_or_else(missing)?;
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            display,
        ))),
        raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
    };
    let (gpu, surface) = unsafe { GpuDevice::shared_for(target, allow_software) }?;
    let capabilities = surface.get_capabilities(gpu.adapter());
    let format = capabilities
        .formats
        .iter()
        .copied()
        .find(|format| {
            matches!(
                format,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
            )
        })
        .ok_or_else(|| DrawError::Gpu("the surface offers no 8-bit UNORM format".to_owned()))?;
    let alpha_mode = if capabilities
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::PreMultiplied)
    {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else {
        wgpu::CompositeAlphaMode::Auto
    };
    let present_mode = if capabilities
        .present_modes
        .contains(&wgpu::PresentMode::Mailbox)
    {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    };
    if !capabilities.usages.contains(wgpu::TextureUsages::COPY_DST) {
        return Err(DrawError::Gpu(
            "the surface cannot receive copied frames".to_owned(),
        ));
    }
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::COPY_DST,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: 1,
        height: 1,
        present_mode,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: Vec::new(),
    };
    Ok(GpuSurface {
        transparent: alpha_mode == wgpu::CompositeAlphaMode::PreMultiplied,
        renderer: GpuRenderer::with_device(Arc::clone(&gpu), format, atlas),
        gpu,
        surface,
        config,
        configured: None,
    })
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::GlobalRemove { name } = event {
            state.outputs.retain(|output| *output != name);
            return;
        }
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(5), handle, ()));
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, handle, ())),
                "xdg_wm_base" => {
                    state.wm_base = Some(registry.bind(name, version.min(6), handle, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(5), handle, ()));
                }
                "xdg_activation_v1" => {
                    state.activation = Some(registry.bind(name, 1, handle, ()));
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(9), handle, ()));
                }
                "wl_data_device_manager" => {
                    state.data_device_manager =
                        Some(registry.bind(name, version.min(3), handle, ()));
                }
                "zwp_primary_selection_device_manager_v1" => {
                    state.clipboard.primary.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_cursor_shape_manager_v1" => {
                    state.cursor_shape_manager = Some(registry.bind(name, 1, handle, ()));
                }
                "ext_session_lock_manager_v1" => {
                    state.lock.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wl_output" => {
                    state.outputs.push(name);
                    if state.lock.output.is_none() {
                        state.lock.output = Some(registry.bind(name, 1, handle, ()));
                    }
                    if version >= 4 && state.lock.wanted_name.is_some() {
                        let _: wayland_client::protocol::wl_output::WlOutput =
                            registry.bind(name, 4, handle, crate::lock::NamedOutput);
                    }
                }
                "unbound_shell_v1" => {
                    state.shell.protocol = Some(registry.bind(name, 1, handle, ()));
                }
                "unbound_app_v1" => {
                    state.shell.app = Some(registry.bind(name, 1, handle, ()));
                }
                "unbound_app_control_v1" => {
                    state.shell.app_control_global = Some((registry.clone(), name));
                }
                "zwp_text_input_manager_v3" => {
                    state.text_input.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_fractional_scale_manager_v1" => {
                    state.scaling.fractional_manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_presentation" => {
                    state.presentation.manager = Some(registry.bind(name, 1, handle, ()));
                }
                "wp_viewporter" => {
                    state.scaling.viewporter = Some(registry.bind(name, 1, handle, ()));
                }
                "ext_foreign_toplevel_list_v1" => {
                    let _: ExtForeignToplevelListV1 = registry.bind(name, 1, handle, ());
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } => {
                if width > 0 && height > 0 {
                    state.suggested_size = Some((width.unsigned_abs(), height.unsigned_abs()));
                }
                let states = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter_map(|bytes| {
                        xdg_toplevel::State::try_from(u32::from_ne_bytes(*bytes)).ok()
                    })
                    .collect::<Vec<_>>();
                let previous = state.toplevel;
                state.toplevel = ToplevelState {
                    maximized: states.contains(&xdg_toplevel::State::Maximized),
                    fullscreen: states.contains(&xdg_toplevel::State::Fullscreen),
                    activated: states.contains(&xdg_toplevel::State::Activated),
                    suspended: states.contains(&xdg_toplevel::State::Suspended),
                    tiled: states.iter().any(|state| {
                        matches!(
                            state,
                            xdg_toplevel::State::TiledLeft
                                | xdg_toplevel::State::TiledRight
                                | xdg_toplevel::State::TiledTop
                                | xdg_toplevel::State::TiledBottom
                        )
                    }),
                };
                if state.toplevel != previous {
                    state
                        .events
                        .push(WindowEvent::ToplevelChanged(state.toplevel));
                }
            }
            xdg_toplevel::Event::Close => state.events.push(WindowEvent::CloseRequested),
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer_surface.ack_configure(serial);
                state.configure(width, height);
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.frame_pending = false;
        }
    }
}

impl Dispatch<WlBuffer, Arc<AtomicBool>> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        event: wl_buffer::Event,
        busy: &Arc<AtomicBool>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            busy.store(false, Ordering::Release);
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlRegion);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore ZwlrLayerShellV1);
delegate_noop!(State: ignore XdgActivationV1);
delegate_noop!(State: ignore WlDataDeviceManager);
