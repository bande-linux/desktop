use std::time::Instant;

use unbound_graphics::{
    Appearance, Color, CpuRenderer, Elevation, Event, Item, Palette, Point, Rect, Scene, Shape,
    TextSystem,
};
use unbound_widgets::{MENU_RADIUS, Menu, MenuOutcome, MenuPanel, MenuTracker};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

use crate::window::{ShmTarget, State, WindowError};

const MARGIN: f32 = 24.0;
const SUBMENU_OVERLAP: f32 = 2.0;
const SUBMENU_LIFT: i32 = -4;
const BELOW_GAP: f32 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuPlacement {
    AtPoint(Point),
    Below(Rect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PopupLevel(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PopupPlacement {
    beside_left: bool,
    height: i32,
}

pub(crate) enum Parent<'a> {
    Toplevel(&'a XdgSurface),
    Layer(&'a ZwlrLayerSurfaceV1),
    Popup(&'a XdgSurface),
}

struct Placement {
    anchor_rect: Rect,
    anchor: Anchor,
    gravity: Gravity,
    offset: (i32, i32),
    adjustment: ConstraintAdjustment,
}

impl Placement {
    fn menu(placement: MenuPlacement) -> Self {
        let fitting = ConstraintAdjustment::FlipY
            | ConstraintAdjustment::SlideX
            | ConstraintAdjustment::SlideY
            | ConstraintAdjustment::ResizeY;
        match placement {
            MenuPlacement::AtPoint(point) => Self {
                anchor_rect: Rect::new(point.x, point.y, 1.0, 1.0),
                anchor: Anchor::TopLeft,
                gravity: Gravity::BottomRight,
                offset: (0, 0),
                adjustment: fitting | ConstraintAdjustment::FlipX,
            },
            MenuPlacement::Below(rect) => Self {
                anchor_rect: Rect::new(
                    rect.x,
                    rect.y - BELOW_GAP,
                    rect.width,
                    rect.height + 2.0 * BELOW_GAP,
                ),
                anchor: Anchor::BottomLeft,
                gravity: Gravity::BottomRight,
                offset: (0, 0),
                adjustment: fitting,
            },
        }
    }

    fn submenu(row: Rect) -> Self {
        Self {
            anchor_rect: Rect::new(
                row.x + SUBMENU_OVERLAP,
                row.y,
                row.width - 2.0 * SUBMENU_OVERLAP,
                row.height,
            ),
            anchor: Anchor::TopRight,
            gravity: Gravity::BottomRight,
            offset: (0, SUBMENU_LIFT),
            adjustment: ConstraintAdjustment::FlipX
                | ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY
                | ConstraintAdjustment::ResizeY,
        }
    }
}

struct Popup {
    surface: WlSurface,
    xdg_surface: XdgSurface,
    popup: XdgPopup,
    viewport: Option<WpViewport>,
    target: ShmTarget,
    shown: Option<Look>,
}

#[derive(Clone, Copy, PartialEq)]
struct Look {
    revision: u32,
    highlighted: Option<usize>,
    alternates: bool,
    show_highlight: bool,
    opacity: f32,
    appearance: Appearance,
    scale: f64,
    scrolled: f32,
    height: f32,
}

impl Popup {
    fn destroy(mut self) {
        self.popup.destroy();
        self.xdg_surface.destroy();
        if let Some(viewport) = &self.viewport {
            viewport.destroy();
        }
        self.target.destroy();
        self.surface.destroy();
    }
}

pub(crate) struct Menus {
    tracker: MenuTracker,
    popups: Vec<Popup>,
}

pub(crate) struct Opening<'a> {
    pub(crate) state: &'a mut State,
    pub(crate) handle: &'a QueueHandle<State>,
    pub(crate) text: &'a mut TextSystem,
    pub(crate) palette: Palette,
}

impl Menus {
    pub(crate) fn open(
        opening: Opening<'_>,
        menu: Menu,
        parent: Parent<'_>,
        placement: MenuPlacement,
    ) -> Result<Self, WindowError> {
        let panel = MenuPanel::new(menu, opening.text, &opening.palette);
        let size = panel.size();
        let mut menus = Self {
            tracker: MenuTracker::new(panel, opening.state.input.keyboard_initiated),
            popups: Vec::new(),
        };
        let popup = create(
            opening.state,
            opening.handle,
            parent,
            Placement::menu(placement),
            (size.width, size.height),
            0,
        )?;
        menus.popups.push(popup);
        Ok(menus)
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.tracker.deadline()
    }

    pub(crate) fn tracker(&self) -> &MenuTracker {
        &self.tracker
    }

    pub(crate) fn handle(&mut self, level: usize, event: &Event, now: Instant) -> MenuOutcome {
        let local = |point: Point| Point::new(point.x - MARGIN, point.y - MARGIN);
        match event {
            Event::PointerMove(pointer) => {
                self.tracker
                    .pointer_move(level, local(pointer.position), now)
            }
            Event::PointerLeave => self
                .tracker
                .pointer_move(level, Point::new(-1.0, -1.0), now),
            Event::PointerDown(pointer) => {
                self.tracker.pointer_press(level, local(pointer.position))
            }
            Event::PointerUp(pointer) => {
                self.tracker
                    .pointer_release(level, local(pointer.position), now)
            }
            Event::Scroll(scroll) => self.tracker.scroll(level, scroll.delta.y),
            Event::Key { key, modifiers } => self.tracker.key(*key, *modifiers, now),
            Event::ModifiersChanged(modifiers) => self.tracker.show_alternates(modifiers.alt),
            _ => MenuOutcome::Nothing,
        }
    }

    pub(crate) fn tick(&mut self, now: Instant) -> MenuOutcome {
        self.tracker.tick(now)
    }

    pub(crate) fn activate(&mut self, level: usize, index: usize, now: Instant) -> MenuOutcome {
        self.tracker.activate(level, index, now)
    }

    pub(crate) fn open_submenu(
        &mut self,
        opening: Opening<'_>,
        level: usize,
        index: usize,
        from_keyboard: bool,
    ) -> Result<(), WindowError> {
        let Some(menu) = self.tracker.submenu(level, index) else {
            return Ok(());
        };
        self.close_from(opening.state, level + 1);
        let Some(parent) = self.popups.get(level) else {
            return Ok(());
        };
        let panel = MenuPanel::new(menu, opening.text, &opening.palette);
        let size = panel.size();
        let popup = create(
            opening.state,
            opening.handle,
            Parent::Popup(&parent.xdg_surface),
            Placement::submenu(self.tracker.panels()[level].row_rect(index)),
            (size.width, size.height),
            level + 1,
        )?;
        self.popups.push(popup);
        self.tracker.opened_submenu(panel, from_keyboard);
        Ok(())
    }

    pub(crate) fn search(&mut self, opening: Opening<'_>, menu: Menu) -> Result<(), WindowError> {
        self.close_from(opening.state, 1);
        let panel = MenuPanel::new(menu, opening.text, &opening.palette);
        self.tracker.searched(panel);
        let size = self.tracker.panels()[0].size();
        let compositor = opening
            .state
            .compositor
            .as_ref()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        if let Some(popup) = self.popups.first() {
            set_shape(
                compositor,
                opening.handle,
                &popup.surface,
                &popup.xdg_surface,
                (size.width, size.height),
            );
        }
        Ok(())
    }

    pub(crate) fn close_from(&mut self, state: &mut State, level: usize) {
        let level = level.max(1);
        for popup in self.popups.drain(level.min(self.popups.len())..).rev() {
            popup.destroy();
        }
        state.popup_surfaces.truncate(level);
        self.tracker.truncate(level);
    }

    pub(crate) fn close(self, state: &mut State) {
        for popup in self.popups.into_iter().rev() {
            popup.destroy();
        }
        state.popup_surfaces.clear();
        state.input.forget_popups();
    }

    fn fit_placements(
        &mut self,
        state: &mut State,
        handle: &QueueHandle<State>,
    ) -> Result<(), WindowError> {
        for level in 0..self.popups.len() {
            let Some(placement) = state.popup_placements.get_mut(level).and_then(Option::take)
            else {
                continue;
            };
            self.tracker
                .placed(level, placement.height as f32, placement.beside_left);
            let size = self.tracker.panels()[level].size();
            let compositor = state
                .compositor
                .as_ref()
                .ok_or(WindowError::Missing("wl_compositor"))?;
            let popup = &self.popups[level];
            set_shape(
                compositor,
                handle,
                &popup.surface,
                &popup.xdg_surface,
                (size.width, size.height),
            );
        }
        Ok(())
    }

    pub(crate) fn draw(
        &mut self,
        state: &mut State,
        handle: &QueueHandle<State>,
        cpu: &mut CpuRenderer,
        appearance: Appearance,
        scale: f64,
        now: Instant,
    ) -> Result<(), WindowError> {
        self.fit_placements(state, handle)?;
        let palette = appearance.palette();
        let shm = state.shm.clone().ok_or(WindowError::Missing("wl_shm"))?;
        for (level, (popup, panel)) in self
            .popups
            .iter_mut()
            .zip(self.tracker.panels())
            .enumerate()
        {
            if !state.popup_configured.get(level).copied().unwrap_or(false)
                || state.popup_frames.get(level).copied().unwrap_or(false)
            {
                continue;
            }
            let (show_highlight, opacity) = self.tracker.appearance(level, now);
            let look = Look {
                revision: self.tracker.revision(),
                highlighted: panel.highlighted(),
                alternates: panel.shows_alternates(),
                show_highlight,
                opacity,
                appearance,
                scale,
                scrolled: panel.scroll_offset(),
                height: panel.size().height,
            };
            if self.tracker.is_flashing() {
                popup.surface.frame(handle, PopupLevel(level));
                if let Some(pending) = state.popup_frames.get_mut(level) {
                    *pending = true;
                }
            }
            if popup.shown == Some(look) {
                if self.tracker.is_flashing() {
                    popup.surface.commit();
                }
                continue;
            }
            let opaque = Look {
                opacity: 1.0,
                ..look
            };
            let only_fading = popup.shown.is_some_and(|shown| {
                Look {
                    opacity: 1.0,
                    ..shown
                } == opaque
            });
            popup.shown = Some(look);
            if only_fading {
                popup
                    .target
                    .present_faded(&shm, handle, &popup.surface, opacity)?;
                continue;
            }
            let size = panel.size();
            let shape = Shape::rounded(Rect::new(0.0, 0.0, size.width, size.height), MENU_RADIUS);
            let mut items = Item::shadows(shape, Elevation::Overlay, palette.scheme, true).to_vec();
            items.extend(panel.paint(&palette, show_highlight));
            let scene = Scene {
                width: (size.width + 2.0 * MARGIN) as u32,
                height: (size.height + 2.0 * MARGIN) as u32,
                background: Color::TRANSPARENT,
                items: vec![Item::Translate {
                    offset: Point::new(MARGIN, MARGIN),
                    items,
                }],
            };
            if let Some(viewport) = &popup.viewport {
                viewport.set_destination(scene.width as i32, scene.height as i32);
            }
            match opacity < 1.0 {
                true => {
                    popup.target.render(cpu, &scene, scale, None)?;
                    popup
                        .target
                        .present_faded(&shm, handle, &popup.surface, opacity)?;
                }
                false => {
                    popup
                        .target
                        .present(cpu, &shm, handle, &popup.surface, &scene, scale, None)?;
                }
            }
        }
        Ok(())
    }
}

fn create(
    state: &mut State,
    handle: &QueueHandle<State>,
    parent: Parent<'_>,
    placement: Placement,
    size: (f32, f32),
    level: usize,
) -> Result<Popup, WindowError> {
    let compositor = state
        .compositor
        .as_ref()
        .ok_or(WindowError::Missing("wl_compositor"))?;
    let wm_base = state
        .wm_base
        .as_ref()
        .ok_or(WindowError::Missing("xdg_wm_base"))?;
    let whole = |value: f32| value.round() as i32;
    let positioner = wm_base.create_positioner(handle, ());
    positioner.set_size(whole(size.0).max(1), whole(size.1).max(1));
    let anchor_rect = placement.anchor_rect;
    positioner.set_anchor_rect(
        whole(anchor_rect.x),
        whole(anchor_rect.y),
        whole(anchor_rect.width).max(1),
        whole(anchor_rect.height).max(1),
    );
    positioner.set_anchor(placement.anchor);
    positioner.set_gravity(placement.gravity);
    positioner.set_offset(placement.offset.0, placement.offset.1);
    positioner.set_constraint_adjustment(placement.adjustment);
    let surface = compositor.create_surface(handle, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, handle, PopupLevel(level));
    let popup = match parent {
        Parent::Toplevel(parent) | Parent::Popup(parent) => {
            xdg_surface.get_popup(Some(parent), &positioner, handle, PopupLevel(level))
        }
        Parent::Layer(layer) => {
            let popup = xdg_surface.get_popup(None, &positioner, handle, PopupLevel(level));
            layer.get_popup(&popup);
            popup
        }
    };
    positioner.destroy();
    set_shape(compositor, handle, &surface, &xdg_surface, size);
    if let (Some(seat), Some(serial)) = (&state.seat, state.input.serial) {
        popup.grab(seat, serial);
    }
    let viewport = state
        .scaling
        .viewporter
        .as_ref()
        .map(|viewporter| viewporter.get_viewport(&surface, handle, ()));
    state.popup_surfaces.truncate(level);
    state.popup_surfaces.push(surface.clone());
    state.popup_configured.truncate(level);
    state.popup_configured.push(false);
    state.popup_frames.truncate(level);
    state.popup_frames.push(false);
    state.popup_placements.truncate(level);
    state.popup_placements.push(None);
    surface.commit();
    Ok(Popup {
        surface,
        xdg_surface,
        popup,
        viewport,
        target: ShmTarget::default(),
        shown: None,
    })
}

fn set_shape(
    compositor: &WlCompositor,
    handle: &QueueHandle<State>,
    surface: &WlSurface,
    xdg_surface: &XdgSurface,
    size: (f32, f32),
) {
    let whole = |value: f32| value.round() as i32;
    let (x, y) = (whole(MARGIN), whole(MARGIN));
    let (width, height) = (whole(size.0).max(1), whole(size.1).max(1));
    xdg_surface.set_window_geometry(x, y, width, height);
    let panel = compositor.create_region(handle, ());
    panel.add(x, y, width, height);
    surface.set_input_region(Some(&panel));
    panel.destroy();
}

impl Dispatch<XdgSurface, PopupLevel> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        level: &PopupLevel,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if let Some(configured) = state.popup_configured.get_mut(level.0) {
                *configured = true;
            }
            state.popups_changed = true;
        }
    }
}

impl Dispatch<XdgPopup, PopupLevel> for State {
    fn event(
        state: &mut Self,
        _: &XdgPopup,
        event: xdg_popup::Event,
        level: &PopupLevel,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::PopupDone => state.popup_dismissed = true,
            xdg_popup::Event::Configure { x, height, .. } => {
                if let Some(placement) = state.popup_placements.get_mut(level.0) {
                    *placement = Some(PopupPlacement {
                        beside_left: x < 0,
                        height,
                    });
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, PopupLevel> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        level: &PopupLevel,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event
            && let Some(pending) = state.popup_frames.get_mut(level.0)
        {
            *pending = false;
            state.popups_changed = true;
        }
    }
}

delegate_noop!(State: ignore XdgPositioner);
