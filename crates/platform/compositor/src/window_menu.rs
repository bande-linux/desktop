use std::time::{Duration, Instant};

use bande_graphics::{Key, Modifiers, Palette, Scheme, TextSize, TextSystem, WindowControl};
use bande_protocols::server::bande_app_v1::Placement;
use bande_render::VulkanRenderer;
use bande_spring::Spring;
use bande_widgets::{MENU_SHADOW_MARGIN, Menu, MenuItem, MenuOutcome, MenuPanel, MenuTracker};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::input::keyboard::{Keysym, keysyms};
use smithay::input::pointer::CursorIcon;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Serial, Size};
use smithay::wayland::shell::xdg::PositionerState;

use crate::L10N;
use crate::label::Labels;
use crate::motion::{Look, Motion, Tick};
use crate::state::{Compositor, OutputElement, State};

const TILING_MENU_AFTER: Duration = Duration::from_millis(500);
const BELOW_GAP: i32 = 4;
const SUBMENU_OVERLAP: i32 = 2;
const SUBMENU_LIFT: i32 = -4;
const SLIDE: f64 = 3.0;

type Frame = Rectangle<f64, Logical>;

#[derive(Debug, Clone, Copy, PartialEq)]
struct PanelLook {
    revision: u32,
    highlighted: Option<usize>,
    show_highlight: bool,
    scrolled: f32,
    height: f32,
    scale: f64,
    scheme: Scheme,
}

struct Panel {
    rect: Frame,
    motion: Motion,
    painted: Option<(PanelLook, MemoryRenderBuffer)>,
}

struct Open {
    window: WlSurface,
    screen: usize,
    tracker: MenuTracker,
    panels: Vec<Panel>,
    closing: bool,
}

#[derive(Default)]
pub(crate) struct WindowMenus {
    open: Option<Open>,
    labels: Option<Labels>,
    text_size: TextSize,
    on_zoom: Option<WlSurface>,
    rest: Option<(WlSurface, Instant)>,
}

impl WindowMenus {
    pub(crate) fn is_open(&self) -> bool {
        self.open.as_ref().is_some_and(|open| !open.closing)
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.open.as_ref().is_some_and(|open| {
            open.tracker.is_flashing() || open.panels.iter().any(|panel| !panel.motion.is_settled())
        })
    }
}

fn menu_text(labels: &mut Option<Labels>, size: TextSize) -> &mut TextSystem {
    let text = labels.get_or_insert_with(Labels::default).text();
    text.set_size(size);
    text
}

enum Anchored {
    AtPoint(Point<f64, Logical>),
    Below(Rectangle<i32, Logical>),
}

fn placed(
    size: Size<i32, Logical>,
    anchor_rect: Rectangle<i32, Logical>,
    anchor_edges: Anchor,
    offset: (i32, i32),
    constraint_adjustment: ConstraintAdjustment,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    PositionerState {
        rect_size: size,
        anchor_rect,
        anchor_edges,
        gravity: Gravity::BottomRight,
        constraint_adjustment,
        offset: offset.into(),
        ..PositionerState::default()
    }
    .get_unconstrained_geometry(area)
}

fn root_placement(
    anchored: &Anchored,
    size: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let fitting = ConstraintAdjustment::FlipY
        | ConstraintAdjustment::SlideX
        | ConstraintAdjustment::SlideY
        | ConstraintAdjustment::ResizeY;
    match anchored {
        Anchored::AtPoint(point) => placed(
            size,
            Rectangle::new(point.to_i32_round(), (1, 1).into()),
            Anchor::TopLeft,
            (0, 0),
            fitting | ConstraintAdjustment::FlipX,
            area,
        ),
        Anchored::Below(rect) => placed(
            size,
            Rectangle::new(
                (rect.loc.x, rect.loc.y - BELOW_GAP).into(),
                (rect.size.w, rect.size.h + 2 * BELOW_GAP).into(),
            ),
            Anchor::BottomLeft,
            (0, 0),
            fitting,
            area,
        ),
    }
}

fn submenu_placement(
    parent: Frame,
    row: bande_graphics::Rect,
    size: Size<i32, Logical>,
    area: Rectangle<i32, Logical>,
) -> Rectangle<i32, Logical> {
    let origin = parent.loc.to_i32_round::<i32>();
    placed(
        size,
        Rectangle::new(
            (
                origin.x + row.x.round() as i32 + SUBMENU_OVERLAP,
                origin.y + row.y.round() as i32,
            )
                .into(),
            (
                (row.width.round() as i32 - 2 * SUBMENU_OVERLAP).max(1),
                (row.height.round() as i32).max(1),
            )
                .into(),
        ),
        Anchor::TopRight,
        (0, SUBMENU_LIFT),
        ConstraintAdjustment::FlipX
            | ConstraintAdjustment::SlideX
            | ConstraintAdjustment::SlideY
            | ConstraintAdjustment::ResizeY,
        area,
    )
}

fn whole_size(panel: &MenuPanel) -> Size<i32, Logical> {
    let size = panel.size();
    (size.width.ceil() as i32, size.height.ceil() as i32).into()
}

fn entering(rect: Frame, toward: (f64, f64), reduce_motion: bool) -> Motion {
    let from = match reduce_motion {
        true => Look::hidden(rect),
        false => Look::hidden(Rectangle::new(
            rect.loc + Point::from((toward.0 * SLIDE, toward.1 * SLIDE)),
            rect.size,
        )),
    };
    Motion::new(Spring::FADE, from, Look::shown(rect))
}

fn menu_key(keysym: u32) -> Option<Key> {
    Some(match keysym {
        keysyms::KEY_Up | keysyms::KEY_KP_Up => Key::Up,
        keysyms::KEY_Down | keysyms::KEY_KP_Down => Key::Down,
        keysyms::KEY_Left | keysyms::KEY_KP_Left => Key::Left,
        keysyms::KEY_Right | keysyms::KEY_KP_Right => Key::Right,
        keysyms::KEY_Home | keysyms::KEY_KP_Home => Key::Home,
        keysyms::KEY_End | keysyms::KEY_KP_End => Key::End,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Key::Enter,
        keysyms::KEY_space => Key::Space,
        keysyms::KEY_Escape => Key::Escape,
        keysyms::KEY_BackSpace => Key::Backspace,
        _ => Key::Character(
            Keysym::new(keysym)
                .key_char()
                .filter(|character| !character.is_control())?,
        ),
    })
}

fn item(title: String, action: &str) -> MenuItem {
    MenuItem::new(&title, action)
}

impl State {
    fn tiling_items(&self, index: usize) -> Vec<MenuItem> {
        let window = &self.windows[index];
        let zoomed = window.has_state(xdg_toplevel::State::Maximized);
        let mut items = vec![
            item(L10N.tr("place-left"), "place-left").shortcut("Super+Left"),
            item(L10N.tr("place-right"), "place-right").shortcut("Super+Right"),
            MenuItem::separator(),
            item(L10N.tr("place-top-left"), "place-top-left"),
            item(L10N.tr("place-top-right"), "place-top-right"),
            item(L10N.tr("place-bottom-left"), "place-bottom-left"),
            item(L10N.tr("place-bottom-right"), "place-bottom-right"),
            MenuItem::separator(),
            item(L10N.tr("place-fill"), "place-fill").enabled(!zoomed),
        ];
        let displays = self
            .screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .count();
        if displays > 1 {
            items.push(
                item(L10N.tr("place-next-display"), "place-next-display")
                    .shortcut("Shift+Super+Right"),
            );
        }
        items.extend([
            MenuItem::separator(),
            item(L10N.tr("place-restore"), "place-restore")
                .enabled(window.tile.is_some() || zoomed),
        ]);
        items
    }

    fn desktop_items(&self, index: usize) -> Vec<MenuItem> {
        let window = &self.windows[index];
        let workspaces = &self.screens[self.screen_of(window)].workspaces;
        let current = workspaces.index_of(window.workspace);
        let mut items: Vec<MenuItem> = (0..workspaces.count())
            .filter(|position| {
                workspaces
                    .id_at(*position)
                    .is_some_and(|id| !workspaces.is_full_screen(id))
            })
            .map(|position| {
                let title = L10N.tr_args("desktop-number", [("number", (position + 1).into())]);
                item(title, &format!("desktop-{position}"))
                    .checked(Some(position) == current)
                    .enabled(Some(position) != current)
            })
            .collect();
        items.extend([
            MenuItem::separator(),
            item(
                L10N.tr("new-desktop"),
                &format!("desktop-{}", workspaces.count()),
            ),
        ]);
        items
    }

    fn window_menu(&self, index: usize) -> Menu {
        let zoomed = self.windows[index].has_state(xdg_toplevel::State::Maximized);
        Menu::new(vec![
            item(L10N.tr("minimize"), "minimize").shortcut("Super+H"),
            item(
                match zoomed {
                    true => L10N.tr("unzoom"),
                    false => L10N.tr("zoom"),
                },
                "zoom",
            ),
            MenuItem::separator(),
            MenuItem::submenu(
                &L10N.tr("move-and-resize"),
                Menu::new(self.tiling_items(index)),
            ),
            MenuItem::submenu(
                &L10N.tr("move-to-desktop"),
                Menu::new(self.desktop_items(index)),
            ),
            MenuItem::separator(),
            item(L10N.tr("close"), "close").shortcut("Alt+F4"),
        ])
    }

    pub(crate) fn open_window_menu(&mut self, surface: &WlSurface, at: Point<f64, Logical>) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let menu = self.window_menu(index);
        self.open_menu(index, menu, Anchored::AtPoint(at), false);
    }

    pub(crate) fn show_client_window_menu(
        &mut self,
        surface: &WlSurface,
        seat: &WlSeat,
        serial: Serial,
        location: Point<i32, Logical>,
    ) {
        if self.grab_start(seat, serial, surface).is_none() {
            return;
        }
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let at = (self.windows[index].surface_origin() + location).to_f64();
        self.open_window_menu(surface, at);
    }

    fn open_tiling_menu(&mut self, index: usize) {
        let window = &self.windows[index];
        let Some(geometry) = window.geometry() else {
            return;
        };
        let control = WindowControl::Zoom.area(window.title_bar_height() as f32);
        let below = Rectangle {
            loc: (
                geometry.loc.x + control.x.round() as i32,
                geometry.loc.y + control.y.round() as i32,
            )
                .into(),
            size: (control.width.round() as i32, control.height.round() as i32).into(),
        };
        let menu = Menu::new(self.tiling_items(index));
        self.open_menu(index, menu, Anchored::Below(below), false);
    }

    fn open_menu(&mut self, index: usize, menu: Menu, anchored: Anchored, from_keyboard: bool) {
        let window = self.windows[index].surface().clone();
        let screen = self.screen_of(&self.windows[index]);
        let area = self.usable_area(screen);
        let palette = Palette::for_scheme(self.decorations.scheme);
        let menus = &mut self.window_menus;
        let panel = MenuPanel::new(
            menu,
            menu_text(&mut menus.labels, menus.text_size),
            &palette,
        );
        let geometry = root_placement(&anchored, whole_size(&panel), area);
        let mut tracker = MenuTracker::new(panel, from_keyboard);
        tracker.placed(0, geometry.size.h as f32, false);
        let rect = geometry.to_f64();
        let toward = match anchored {
            Anchored::AtPoint(point) if point.y > rect.loc.y + rect.size.h / 2.0 => (0.0, 1.0),
            Anchored::AtPoint(_) | Anchored::Below(_) => (0.0, -1.0),
        };
        self.window_menus.open = Some(Open {
            window,
            screen,
            tracker,
            panels: vec![Panel {
                rect,
                motion: entering(rect, toward, self.reduce_motion),
                painted: None,
            }],
            closing: false,
        });
        self.window_menus.rest = None;
        self.window_menus.on_zoom = None;
        for window in &mut self.windows {
            window.frame.hovered = None;
            window.frame.pressed = None;
        }
        self.show_cursor(CursorIcon::Default);
        self.needs_redraw = true;
    }

    fn open_submenu(&mut self, level: usize, index: usize, from_keyboard: bool) {
        let palette = Palette::for_scheme(self.decorations.scheme);
        let reduce_motion = self.reduce_motion;
        let Some(screen) = self.window_menus.open.as_ref().map(|open| open.screen) else {
            return;
        };
        let area = self.usable_area(screen);
        let WindowMenus {
            open,
            labels,
            text_size,
            ..
        } = &mut self.window_menus;
        let Some(open) = open.as_mut() else {
            return;
        };
        let Some(menu) = open.tracker.submenu(level, index) else {
            return;
        };
        let Some(parent) = open.panels.get(level).map(|panel| panel.rect) else {
            return;
        };
        open.panels.truncate(level + 1);
        open.tracker.truncate(level + 1);
        let row = open.tracker.panels()[level].row_rect(index);
        let panel = MenuPanel::new(menu, menu_text(labels, *text_size), &palette);
        let geometry = submenu_placement(parent, row, whole_size(&panel), area);
        let beside_left = f64::from(geometry.loc.x) < parent.loc.x;
        open.tracker.opened_submenu(panel, from_keyboard);
        open.tracker
            .placed(level + 1, geometry.size.h as f32, beside_left);
        let rect = geometry.to_f64();
        let toward = if beside_left { (1.0, 0.0) } else { (-1.0, 0.0) };
        open.panels.push(Panel {
            rect,
            motion: entering(rect, toward, reduce_motion),
            painted: None,
        });
        self.needs_redraw = true;
    }

    pub(crate) fn dismiss_window_menu(&mut self) {
        let Some(open) = self.window_menus.open.as_mut().filter(|open| !open.closing) else {
            return;
        };
        open.closing = true;
        for panel in &mut open.panels {
            panel.motion.retarget(Look::hidden(panel.rect));
        }
        self.needs_redraw = true;
        self.refocus_pointer();
    }

    pub(crate) fn dismiss_window_menu_unless_on(&mut self, surface: Option<&WlSurface>) {
        if self
            .window_menus
            .open
            .as_ref()
            .is_some_and(|open| Some(&open.window) != surface)
        {
            self.dismiss_window_menu();
        }
    }

    fn apply_menu_outcome(&mut self, outcome: MenuOutcome, from_keyboard: bool) {
        match outcome {
            MenuOutcome::Nothing | MenuOutcome::Search(_) => {}
            MenuOutcome::Redraw => self.needs_redraw = true,
            MenuOutcome::OpenSubmenu { level, index } => {
                self.open_submenu(level, index, from_keyboard);
            }
            MenuOutcome::CloseFrom(level) => {
                if let Some(open) = self.window_menus.open.as_mut() {
                    open.panels.truncate(level.max(1));
                    open.tracker.truncate(level);
                }
                self.needs_redraw = true;
            }
            MenuOutcome::Activate(action) => {
                if let Some(window) = self
                    .window_menus
                    .open
                    .as_ref()
                    .map(|open| open.window.clone())
                {
                    self.window_menu_action(&window, &action);
                }
                self.needs_redraw = true;
            }
            MenuOutcome::Dismiss => self.dismiss_window_menu(),
        }
    }

    fn window_menu_action(&mut self, surface: &WlSurface, action: &str) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let placement = match action {
            "minimize" => return self.minimize(surface),
            "zoom" => return self.toggle_zoom(surface),
            "close" => return self.windows[index].toplevel.send_close(),
            "place-fill" => return self.maximize(surface),
            "place-left" => Placement::Left,
            "place-right" => Placement::Right,
            "place-top-left" => Placement::TopLeft,
            "place-top-right" => Placement::TopRight,
            "place-bottom-left" => Placement::BottomLeft,
            "place-bottom-right" => Placement::BottomRight,
            "place-next-display" => Placement::NextDisplay,
            "place-restore" => Placement::Restore,
            desktop => {
                if let Some(position) = desktop
                    .strip_prefix("desktop-")
                    .and_then(|position| position.parse().ok())
                {
                    let screen = self.screen_of(&self.windows[index]);
                    self.move_to_space_at(surface, screen, position);
                }
                return;
            }
        };
        self.apply_placement(index, placement);
    }

    fn panel_under(&self, location: Point<f64, Logical>) -> Option<(usize, bande_graphics::Point)> {
        let open = self.window_menus.open.as_ref()?;
        open.panels
            .iter()
            .enumerate()
            .rev()
            .find(|(_, panel)| panel.rect.contains(location))
            .map(|(level, panel)| {
                (
                    level,
                    bande_graphics::Point::new(
                        (location.x - panel.rect.loc.x) as f32,
                        (location.y - panel.rect.loc.y) as f32,
                    ),
                )
            })
    }

    pub(crate) fn window_menu_pointer(&mut self, location: Point<f64, Logical>) -> bool {
        if !self.window_menus.is_open() {
            return false;
        }
        let now = Instant::now();
        let outcome = match (self.panel_under(location), self.window_menus.open.as_mut()) {
            (Some((level, point)), Some(open)) => open.tracker.pointer_move(level, point, now),
            (None, Some(open)) => {
                let deepest = open.panels.len().saturating_sub(1);
                open.tracker
                    .pointer_move(deepest, bande_graphics::Point::new(-1.0, -1.0), now)
            }
            (_, None) => MenuOutcome::Nothing,
        };
        self.apply_menu_outcome(outcome, false);
        true
    }

    pub(crate) fn window_menu_button(&mut self, button: u32, pressed: bool) -> bool {
        if !pressed {
            self.window_menus.rest = None;
        }
        if !self.window_menus.is_open() {
            return false;
        }
        if !pressed && self.swallowed_button == Some(button) {
            self.swallowed_button = None;
        }
        let under = self.panel_under(self.pointer_location);
        let Some(open) = self.window_menus.open.as_mut() else {
            return false;
        };
        let outcome = match (under, pressed) {
            (Some((level, point)), true) => open.tracker.pointer_press(level, point),
            (Some((level, point)), false) => {
                open.tracker.pointer_release(level, point, Instant::now())
            }
            (None, true) => {
                self.swallowed_button = Some(button);
                MenuOutcome::Dismiss
            }
            (None, false) => MenuOutcome::Nothing,
        };
        self.apply_menu_outcome(outcome, false);
        let client_holds_button = !pressed
            && self
                .seat
                .get_pointer()
                .is_some_and(|pointer| pointer.is_grabbed());
        !client_holds_button
    }

    pub(crate) fn window_menu_key(&mut self, keysym: u32, modifiers: Modifiers) {
        let Some(key) = menu_key(keysym) else {
            return;
        };
        let Some(open) = self.window_menus.open.as_mut().filter(|open| !open.closing) else {
            return;
        };
        let outcome = open.tracker.key(key, modifiers, Instant::now());
        self.apply_menu_outcome(outcome, true);
    }

    pub(crate) fn rest_on_zoom(&mut self, on_zoom: Option<WlSurface>) {
        if self.window_menus.on_zoom != on_zoom {
            self.window_menus.rest = on_zoom.clone().map(|surface| (surface, Instant::now()));
            self.window_menus.on_zoom = on_zoom;
        }
    }

    pub(crate) fn press_zoom(&mut self, surface: &WlSurface) {
        self.window_menus.rest = Some((surface.clone(), Instant::now()));
    }

    fn offer_tiling_menu(&mut self, now: Instant) {
        let Some((surface, since)) = self.window_menus.rest.clone() else {
            return;
        };
        if now < since + TILING_MENU_AFTER {
            return;
        }
        self.window_menus.rest = None;
        let Some(index) = self.window_index(&surface) else {
            return;
        };
        let frame = &self.windows[index].frame;
        let on_zoom = frame.pressed == Some(WindowControl::Zoom)
            || frame.hovered == Some(Some(WindowControl::Zoom));
        if on_zoom
            && !self.window_menus.is_open()
            && !self.windows[index].has_state(xdg_toplevel::State::Fullscreen)
        {
            self.open_tiling_menu(index);
        }
    }

    fn window_menu_due(&mut self, now: Instant) -> Option<Instant> {
        let gone = self.window_menus.open.as_ref().is_some_and(|open| {
            !open.closing && (self.window_index(&open.window).is_none() || self.is_locked())
        });
        if gone {
            self.dismiss_window_menu();
        }
        self.offer_tiling_menu(now);
        if let Some(open) = self.window_menus.open.as_mut().filter(|open| !open.closing) {
            let outcome = open.tracker.tick(now);
            self.apply_menu_outcome(outcome, false);
        }
        let menu = self
            .window_menus
            .open
            .as_ref()
            .filter(|open| !open.closing)
            .and_then(|open| open.tracker.deadline());
        let rest = self
            .window_menus
            .rest
            .as_ref()
            .map(|(_, since)| *since + TILING_MENU_AFTER);
        menu.into_iter().chain(rest).min()
    }

    pub(crate) fn animate_window_menu(&mut self, tick: Tick) {
        let Some(open) = self.window_menus.open.as_mut() else {
            return;
        };
        for panel in &mut open.panels {
            panel.motion.advance(tick);
        }
        if open.closing && open.panels.iter().all(|panel| panel.motion.is_settled()) {
            self.window_menus.open = None;
            self.needs_redraw = true;
        }
    }

    pub(crate) fn prepare_window_menu(&mut self, screen: usize) {
        let scale = self.screens[screen].scale();
        let scheme = self.decorations.scheme;
        let palette = Palette::for_scheme(scheme);
        let now = Instant::now();
        let WindowMenus { open, labels, .. } = &mut self.window_menus;
        let Some(open) = open.as_mut().filter(|open| open.screen == screen) else {
            return;
        };
        for (level, (panel, shown)) in open
            .tracker
            .panels()
            .iter()
            .zip(&mut open.panels)
            .enumerate()
        {
            let show_highlight = open.tracker.shows_highlight(level, now);
            let look = PanelLook {
                revision: open.tracker.revision(),
                highlighted: panel.highlighted(),
                show_highlight,
                scrolled: panel.scroll_offset(),
                height: panel.size().height,
                scale,
                scheme,
            };
            if shown
                .painted
                .as_ref()
                .is_some_and(|(painted, _)| *painted == look)
            {
                continue;
            }
            let scene = panel.scene(&palette, show_highlight);
            shown.painted = labels
                .get_or_insert_with(Labels::default)
                .paint(&scene, scale)
                .map(|buffer| (look, buffer));
        }
    }

    pub(crate) fn window_menu_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        let Some(open) = self
            .window_menus
            .open
            .as_ref()
            .filter(|open| open.screen == screen && !self.is_locked())
        else {
            return Vec::new();
        };
        let margin = f64::from(MENU_SHADOW_MARGIN);
        open.panels
            .iter()
            .rev()
            .filter_map(|panel| {
                let (look, buffer) = panel.painted.as_ref()?;
                let shown = panel.motion.look();
                let size = Size::<f64, Logical>::from((
                    panel.rect.size.w + 2.0 * margin,
                    f64::from(look.height) + 2.0 * margin,
                ));
                let pixels = (
                    f64::from(bande_graphics::pixels(size.w as u32, look.scale)),
                    f64::from(bande_graphics::pixels(size.h as u32, look.scale)),
                );
                let origin = shown.rect.loc - Point::from((margin, margin));
                MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    origin.to_physical(scale).to_i32_round::<i32>().to_f64(),
                    buffer,
                    Some(shown.alpha as f32),
                    Some(Rectangle::from_size(pixels.into())),
                    Some(size.to_i32_round()),
                    Kind::Unspecified,
                )
                .ok()
                .map(OutputElement::Memory)
            })
            .collect()
    }
}

impl Compositor {
    pub fn set_text_size(&mut self, size: TextSize) {
        self.state.window_menus.text_size = size;
    }

    pub fn window_menu_tick(&mut self, now: Instant) -> Option<Instant> {
        self.state.window_menu_due(now)
    }

    pub fn window_menu(&self) -> Vec<Vec<String>> {
        self.state
            .window_menus
            .open
            .as_ref()
            .filter(|open| !open.closing)
            .map(|open| {
                open.tracker
                    .panels()
                    .iter()
                    .map(|panel| {
                        panel
                            .menu()
                            .items
                            .iter()
                            .filter(|item| !item.separator)
                            .map(|item| item.title.clone())
                            .collect()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn window_menu_item(&self, title: &str) -> Option<Point<f64, Logical>> {
        let open = self.state.window_menus.open.as_ref()?;
        open.tracker
            .panels()
            .iter()
            .zip(&open.panels)
            .rev()
            .find_map(|(panel, shown)| {
                let index = panel
                    .menu()
                    .items
                    .iter()
                    .position(|item| item.title == title)?;
                let row = panel.row_rect(index);
                Some(Point::from((
                    shown.rect.loc.x + f64::from(row.x + row.width / 2.0),
                    shown.rect.loc.y + f64::from(row.y + row.height / 2.0),
                )))
            })
    }
}
