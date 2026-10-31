use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::Id;
use smithay::backend::renderer::{ContextId, ImportMem, Renderer, Texture};
use smithay::desktop::WindowSurfaceType;
use smithay::desktop::utils::under_from_surface_tree;
use smithay::input::pointer::{CursorIcon, CursorImageStatus};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle};
use unbound_graphics::{
    Color, DOUBLE_CLICK_INTERVAL, Palette, RESIZE_MARGIN, Rect, ResizeEdge, Scene, Scheme,
    TextAlign, TitleBar, WindowControl, resize_edge, title_style, window_controls_area,
};
use unbound_render::{VulkanRenderer, VulkanTexture};

use crate::label::Labels;
use crate::scene::Piece;
use crate::shell::toplevel_names;
use crate::state::{Headless, Placed, State};
use crate::window::Window;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Zone {
    Control(WindowControl),
    TitleBar,
    Resize(ResizeEdge),
}

impl Zone {
    fn cursor(self) -> CursorIcon {
        match self {
            Self::Control(_) | Self::TitleBar => CursorIcon::Default,
            Self::Resize(edge) => match edge {
                ResizeEdge::Top => CursorIcon::NResize,
                ResizeEdge::Bottom => CursorIcon::SResize,
                ResizeEdge::Left => CursorIcon::WResize,
                ResizeEdge::Right => CursorIcon::EResize,
                ResizeEdge::TopLeft => CursorIcon::NwResize,
                ResizeEdge::TopRight => CursorIcon::NeResize,
                ResizeEdge::BottomLeft => CursorIcon::SwResize,
                ResizeEdge::BottomRight => CursorIcon::SeResize,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Look {
    title: String,
    width: i32,
    scale: u64,
    active: bool,
    scheme: Scheme,
    hovered: Option<Option<WindowControl>>,
    pressed: Option<WindowControl>,
    zoomed: bool,
    alternate: bool,
}

#[derive(Debug, Clone)]
struct Painted {
    look: Look,
    context: ContextId<VulkanTexture>,
    id: Id,
    texture: VulkanTexture,
}

#[derive(Debug, Default)]
pub(crate) struct Frame {
    pub(crate) hovered: Option<Option<WindowControl>>,
    pub(crate) pressed: Option<WindowControl>,
    painted: Option<Painted>,
}

#[derive(Default)]
pub(crate) struct Decorations {
    pub(crate) scheme: Scheme,
    pointed: Option<(WlSurface, Zone)>,
    last_click: Option<(WlSurface, u32)>,
}

fn zone_at(window: &Window, location: Point<f64, Logical>) -> Option<Zone> {
    let bar = window.title_bar_height();
    if bar == 0 {
        return None;
    }
    let rect = window.geometry()?;
    let point = local_point(rect, location);
    let frame = Rect::new(0.0, 0.0, rect.size.w as f32, rect.size.h as f32);
    if frame.contains(point) {
        if point.y >= bar as f32 {
            return None;
        }
        return Some(WindowControl::at(point, bar as f32).map_or(Zone::TitleBar, Zone::Control));
    }
    if !frame.inset(-(RESIZE_MARGIN as f32)).contains(point)
        || window.has_state(xdg_toplevel::State::Maximized)
        || window.has_state(xdg_toplevel::State::Fullscreen)
    {
        return None;
    }
    Some(Zone::Resize(resize_edge(frame, point)))
}

fn local_point(rect: crate::state::Rect, location: Point<f64, Logical>) -> unbound_graphics::Point {
    unbound_graphics::Point::new(
        (location.x - f64::from(rect.location.x)) as f32,
        (location.y - f64::from(rect.location.y)) as f32,
    )
}

fn protocol_edge(edge: ResizeEdge) -> xdg_toplevel::ResizeEdge {
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

fn hovered_control(
    window: &Window,
    location: Point<f64, Logical>,
) -> Option<Option<WindowControl>> {
    let rect = window.geometry()?;
    let bar = window.title_bar_height() as f32;
    let area = window_controls_area(bar);
    let point = local_point(rect, location);
    area.contains(point).then(|| WindowControl::at(point, bar))
}

pub(crate) enum Target {
    Surface(WlSurface, Point<f64, Logical>),
    Frame(WlSurface, Zone),
    Scrim(WlSurface),
}

impl Target {
    pub(crate) fn surface(&self) -> Option<(WlSurface, Point<f64, Logical>)> {
        match self {
            Target::Surface(surface, at) => Some((surface.clone(), *at)),
            Target::Frame(..) | Target::Scrim(_) => None,
        }
    }

    pub(crate) fn frame(&self) -> Option<(WlSurface, Zone)> {
        match self {
            Target::Frame(surface, zone) => Some((surface.clone(), *zone)),
            Target::Surface(..) | Target::Scrim(_) => None,
        }
    }
}

fn surface_at(
    surface: &WlSurface,
    origin: Point<i32, Logical>,
    location: Point<f64, Logical>,
) -> Option<Target> {
    under_from_surface_tree(surface, location, origin, WindowSurfaceType::ALL)
        .map(|(surface, at)| Target::Surface(surface, at.to_f64()))
}

impl State {
    pub(crate) fn pointer_target(&self, location: Point<f64, Logical>) -> Option<Target> {
        let screen = self.screen_at(location)?;
        self.scene(false, screen)
            .into_iter()
            .find_map(|placed| match placed {
                Placed::Surface(surface, origin) => surface_at(&surface, origin, location),
                Placed::Window(window) => zone_at(window, location)
                    .map(|zone| Target::Frame(window.surface().clone(), zone))
                    .or_else(|| surface_at(window.surface(), window.surface_origin(), location)),
                Placed::Scrim(owner, _) => Some(Target::Scrim(owner)),
                Placed::Closing(_) | Placed::Overview(_) | Placed::Backdrop(_) => None,
            })
    }

    pub(crate) fn point_at_frame(
        &mut self,
        location: Point<f64, Logical>,
        pointed: Option<(WlSurface, Zone)>,
        over_surface: bool,
    ) {
        let hovered = pointed
            .as_ref()
            .filter(|(_, zone)| matches!(zone, Zone::Control(_) | Zone::TitleBar))
            .map(|(surface, _)| surface.clone());
        for window in &mut self.windows {
            let over = hovered
                .as_ref()
                .filter(|surface| *surface == window.surface())
                .and_then(|_| hovered_control(window, location));
            if window.frame.hovered != over {
                window.frame.hovered = over;
                self.needs_redraw = true;
            }
        }
        let on_zoom = self
            .windows
            .iter()
            .find(|window| window.frame.hovered == Some(Some(WindowControl::Zoom)))
            .map(|window| window.surface().clone());
        self.rest_on_zoom(on_zoom);
        let previous = std::mem::replace(&mut self.decorations.pointed, pointed.clone());
        match pointed {
            Some((_, zone)) => {
                if previous.map(|(_, zone)| zone.cursor()) != Some(zone.cursor()) {
                    self.show_cursor(zone.cursor());
                }
            }
            None if previous.is_some() && !over_surface => self.show_cursor(CursorIcon::Default),
            None => {}
        }
    }

    pub(crate) fn show_cursor(&mut self, icon: CursorIcon) {
        self.load_named_cursor(icon);
        self.cursor_status = CursorImageStatus::Named(icon);
        self.needs_redraw = true;
    }

    pub(crate) fn press_frame(
        &mut self,
        (surface, zone): (WlSurface, Zone),
        button: u32,
        time: u32,
    ) -> bool {
        self.focus(Some(surface.clone()));
        if button != BTN_LEFT {
            self.swallowed_button = Some(button);
            if button == BTN_RIGHT && matches!(zone, Zone::Control(_) | Zone::TitleBar) {
                self.open_window_menu(&surface);
            }
            return true;
        }
        match zone {
            Zone::Control(control) => {
                if let Some(index) = self.window_index(&surface) {
                    self.windows[index].frame.pressed = Some(control);
                    self.needs_redraw = true;
                }
                if control == WindowControl::Zoom {
                    self.press_zoom(&surface);
                }
                true
            }
            Zone::TitleBar => {
                let double = self
                    .decorations
                    .last_click
                    .take()
                    .is_some_and(|(clicked, at)| {
                        clicked == surface
                            && Duration::from_millis(u64::from(time.wrapping_sub(at)))
                                <= DOUBLE_CLICK_INTERVAL
                    });
                if double {
                    self.swallowed_button = Some(button);
                    self.toggle_zoom(&surface);
                    return true;
                }
                self.decorations.last_click = Some((surface.clone(), time));
                self.move_from_title_bar(&surface, button);
                false
            }
            Zone::Resize(edges) => {
                self.resize_from_frame(&surface, protocol_edge(edges), button);
                false
            }
        }
    }

    pub(crate) fn release_frame(&mut self, button: u32) -> bool {
        if button != BTN_LEFT {
            return false;
        }
        let Some(index) = self
            .windows
            .iter()
            .position(|window| window.frame.pressed.is_some())
        else {
            return false;
        };
        let control = self.windows[index].frame.pressed.take();
        self.needs_redraw = true;
        let surface = self.windows[index].surface().clone();
        let released_on = self
            .pointer_target(self.pointer_location)
            .and_then(|target| target.frame());
        if released_on.as_ref().map(|(on, zone)| (on, *zone))
            != control.map(|control| (&surface, Zone::Control(control)))
        {
            return true;
        }
        match control {
            Some(WindowControl::Close) => self.windows[index].toplevel.send_close(),
            Some(WindowControl::Minimize) => self.minimize(&surface),
            Some(WindowControl::Zoom) if self.alternate_held() => self.fullscreen(&surface),
            Some(WindowControl::Zoom) => self.toggle_zoom(&surface),
            None => {}
        }
        true
    }

    pub(crate) fn toggle_zoom(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        if self.windows[index].has_state(xdg_toplevel::State::Maximized) {
            self.unmaximize(surface);
        } else {
            self.maximize(surface);
        }
    }

    pub(crate) fn has_server_side_frames(&self) -> bool {
        self.windows
            .iter()
            .any(|window| window.title_bar_height() > 0)
    }

    fn alternate_held(&self) -> bool {
        self.seat
            .get_keyboard()
            .is_some_and(|keyboard| keyboard.modifier_state().alt)
    }

    pub(crate) fn paint_title_bars(&mut self, renderer: &mut VulkanRenderer) {
        let key = self.key_window();
        let alternate = self.alternate_held();
        let scales: Vec<f64> = self
            .windows
            .iter()
            .map(|window| self.screens[self.screen_of(window)].scale())
            .collect();
        let scheme = self.decorations.scheme;
        let palette = Palette::for_scheme(scheme);
        let context = renderer.context_id();
        let labels = self.labels.get_mut();
        for (window, scale) in self.windows.iter_mut().zip(scales) {
            let bar = window.title_bar_height();
            let Some(rect) = window.geometry().filter(|_| bar > 0) else {
                window.frame.painted = None;
                continue;
            };
            let look = Look {
                title: toplevel_names(window.surface()).0,
                width: rect.size.w,
                scale: scale.to_bits(),
                active: key.as_ref() == Some(window.surface()),
                scheme,
                hovered: window.frame.hovered,
                pressed: window.frame.pressed,
                zoomed: window.has_state(xdg_toplevel::State::Maximized),
                alternate,
            };
            if window
                .frame
                .painted
                .as_ref()
                .is_some_and(|painted| painted.look == look && painted.context == context)
            {
                continue;
            }
            match paint_title_bar(labels, &look, bar, &palette, renderer) {
                Some(texture) => {
                    window.frame.painted = Some(Painted {
                        look,
                        context: context.clone(),
                        id: Id::new(),
                        texture,
                    });
                }
                None => window.frame.painted = None,
            }
        }
    }
}

impl Headless {
    pub fn set_dark_appearance(&mut self, dark: bool) {
        self.state.decorations.scheme = if dark { Scheme::Dark } else { Scheme::Light };
        self.state.needs_redraw = true;
    }
}

fn paint_title_bar(
    labels: &mut Labels,
    look: &Look,
    bar: i32,
    palette: &Palette,
    renderer: &mut VulkanRenderer,
) -> Option<VulkanTexture> {
    let title = labels.text().paragraph(
        &look.title,
        title_style(palette, look.active),
        None,
        TextAlign::Start,
    );
    let scene = Scene {
        width: look.width.unsigned_abs(),
        height: bar.unsigned_abs(),
        background: Color::TRANSPARENT,
        items: TitleBar {
            width: look.width as f32,
            height: bar as f32,
            title,
            palette,
            active: look.active,
            hover: if look.hovered.is_some() { 1.0 } else { 0.0 },
            close_hover: if look.hovered == Some(Some(WindowControl::Close)) {
                1.0
            } else {
                0.0
            },
            pressed: look.pressed,
            zoomed: look.zoomed,
            alternate: look.alternate && look.active,
        }
        .items(),
    };
    let (rgba, size) = labels.rasterise(&scene, f64::from_bits(look.scale))?;
    renderer
        .import_memory(&rgba, Fourcc::Abgr8888, size.into(), false)
        .inspect_err(|error| tracing::warn!("cannot upload a title bar: {error}"))
        .ok()
}

impl Window {
    pub(crate) fn title_bar_piece(&self, scale: f64) -> Option<Piece> {
        let painted = self.frame.painted.as_ref()?;
        let bar = self.title_bar_height();
        if bar == 0 {
            return None;
        }
        let texture = (
            f64::from(painted.texture.width()),
            f64::from(painted.texture.height()),
        );
        let size = (
            unbound_graphics::pixels(painted.look.width.unsigned_abs(), scale) as i32,
            unbound_graphics::pixels(bar.unsigned_abs(), scale) as i32,
        );
        let location: Point<i32, Physical> =
            self.location.to_f64().to_physical(scale).to_i32_round();
        Some(Piece {
            id: painted.id.clone(),
            texture: painted.texture.clone(),
            src: Rectangle::<f64, BufferCoords>::from_size(texture.into()),
            geometry: Rectangle::new(location, size.into()),
        })
    }
}
