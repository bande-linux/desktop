use std::time::{Duration, Instant};

use unbound_graphics::{
    Color, CpuRenderer, Event, Item, Palette, Point, Rect, Scene, Shape, TextSystem,
};
use unbound_protocols::client::unbound_shell_v1;
use unbound_widgets::{Menu, MenuOutcome, MenuPanel, MenuTracker};
use wayland_client::Proxy;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

use crate::window::{ShmTarget, State, WindowError};

const MARGIN: f32 = 24.0;
const RADIUS: f32 = 8.0;
const BLUR_BANDS: usize = 4;
const TRANSLUCENT_ALPHA: u8 = 200;
const OPAQUE_ALPHA: u8 = 250;
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuPlacement {
    AtPoint(Point),
    Below(Rect),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PopupLevel(pub(crate) usize);

pub(crate) enum Parent<'a> {
    Toplevel(&'a XdgSurface),
    Layer(&'a ZwlrLayerSurfaceV1),
    Popup(&'a XdgSurface),
}

struct Placement {
    anchor_rect: Rect,
    anchor: Anchor,
    offset: (i32, i32),
}

struct Popup {
    surface: WlSurface,
    xdg_surface: XdgSurface,
    popup: XdgPopup,
    viewport: Option<WpViewport>,
    blur: Option<ExtBackgroundEffectSurfaceV1>,
    target: ShmTarget,
    shown: Option<Look>,
}

#[derive(Clone, Copy, PartialEq)]
struct Look {
    highlighted: Option<usize>,
    alternates: bool,
    show_highlight: bool,
    opacity: f32,
    palette: Palette,
    scale: f64,
}

impl Popup {
    fn destroy(mut self) {
        if let Some(blur) = &self.blur {
            blur.destroy();
        }
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
        let placement = match placement {
            MenuPlacement::AtPoint(point) => Placement {
                anchor_rect: Rect::new(point.x, point.y, 1.0, 1.0),
                anchor: Anchor::TopLeft,
                offset: (0, 0),
            },
            MenuPlacement::Below(rect) => Placement {
                anchor_rect: rect,
                anchor: Anchor::BottomLeft,
                offset: (0, 4),
            },
        };
        let popup = create(
            opening.state,
            opening.handle,
            parent,
            placement,
            (size.width, size.height),
            0,
        )?;
        menus.popups.push(popup);
        Ok(menus)
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        let frame = self
            .tracker
            .is_flashing()
            .then(|| Instant::now() + FRAME_INTERVAL);
        self.tracker.deadline().into_iter().chain(frame).min()
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
        let Some(parent) = self.popups.get(level) else {
            return Ok(());
        };
        let panel = MenuPanel::new(menu, opening.text, &opening.palette);
        let size = panel.size();
        let placement = Placement {
            anchor_rect: self.tracker.panels()[level].row_rect(index),
            anchor: Anchor::TopRight,
            offset: (-5, -5),
        };
        let popup = create(
            opening.state,
            opening.handle,
            Parent::Popup(&parent.xdg_surface),
            placement,
            (size.width, size.height),
            level + 1,
        )?;
        self.popups.push(popup);
        self.tracker.opened_submenu(panel, from_keyboard);
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

    pub(crate) fn draw(
        &mut self,
        state: &State,
        handle: &QueueHandle<State>,
        cpu: &mut CpuRenderer,
        palette: &Palette,
        scale: f64,
        now: Instant,
    ) -> Result<(), WindowError> {
        let shm = state.shm.as_ref().ok_or(WindowError::Missing("wl_shm"))?;
        for (level, (popup, panel)) in self
            .popups
            .iter_mut()
            .zip(self.tracker.panels())
            .enumerate()
        {
            if !state.popup_configured.get(level).copied().unwrap_or(false) {
                continue;
            }
            let (show_highlight, opacity) = self.tracker.appearance(level, now);
            let look = Look {
                highlighted: panel.highlighted(),
                alternates: panel.shows_alternates(),
                show_highlight,
                opacity,
                palette: *palette,
                scale,
            };
            if popup.shown == Some(look) {
                continue;
            }
            if opacity < 1.0
                && popup.shown.is_some_and(|shown| shown.opacity >= 1.0)
                && let Some(blur) = &popup.blur
            {
                blur.set_blur_region(None);
            }
            popup.shown = Some(look);
            let size = panel.size();
            let shape = Shape::rounded(Rect::new(0.0, 0.0, size.width, size.height), RADIUS);
            let material = Palette {
                overlay: palette.overlay.with_alpha(if popup.blur.is_some() {
                    TRANSLUCENT_ALPHA
                } else {
                    OPAQUE_ALPHA
                }),
                ..*palette
            };
            let mut items = vec![Item::Shadow {
                shape,
                offset: Point::new(0.0, 6.0),
                blur_radius: 16.0,
                color: Color::hex(0x000000).with_alpha(64),
            }];
            items.extend(panel.paint(&material, show_highlight));
            let scene = Scene {
                width: (size.width + 2.0 * MARGIN) as u32,
                height: (size.height + 2.0 * MARGIN) as u32,
                background: Color::TRANSPARENT,
                items: vec![Item::Opacity {
                    alpha: opacity,
                    items: vec![Item::Translate {
                        offset: Point::new(MARGIN, MARGIN),
                        items,
                    }],
                }],
            };
            if let Some(viewport) = &popup.viewport {
                viewport.set_destination(scene.width as i32, scene.height as i32);
            }
            popup
                .target
                .present(cpu, shm, handle, &popup.surface, &scene, scale)?;
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
    positioner.set_gravity(Gravity::BottomRight);
    positioner.set_offset(placement.offset.0, placement.offset.1);
    positioner.set_constraint_adjustment(
        ConstraintAdjustment::FlipX
            | ConstraintAdjustment::FlipY
            | ConstraintAdjustment::SlideX
            | ConstraintAdjustment::SlideY,
    );
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
    xdg_surface.set_window_geometry(
        whole(MARGIN),
        whole(MARGIN),
        whole(size.0).max(1),
        whole(size.1).max(1),
    );
    if let (Some(seat), Some(serial)) = (&state.seat, state.input.serial) {
        popup.grab(seat, serial);
    }
    let viewport = state
        .scaling
        .viewporter
        .as_ref()
        .map(|viewporter| viewporter.get_viewport(&surface, handle, ()));
    let blur = state.effect.manager.as_ref().map(|manager| {
        let blur = manager.get_background_effect(&surface, handle, ());
        let outline = Rect::new(MARGIN, MARGIN, size.0, size.1);
        let rounded = state
            .shell
            .protocol
            .as_ref()
            .filter(|shell| shell.version() >= unbound_shell_v1::REQ_SET_CORNER_RADIUS_SINCE)
            .map(|shell| shell.set_corner_radius(&surface, RADIUS as u32))
            .is_some();
        let rects = if rounded {
            vec![outline]
        } else {
            Shape::rounded(outline, RADIUS).bands(BLUR_BANDS)
        };
        let region = compositor.create_region(handle, ());
        for rect in rects {
            region.add(
                whole(rect.x),
                whole(rect.y),
                whole(rect.width),
                whole(rect.height),
            );
        }
        blur.set_blur_region(Some(&region));
        region.destroy();
        blur
    });
    state.popup_surfaces.truncate(level);
    state.popup_surfaces.push(surface.clone());
    state.popup_configured.truncate(level);
    state.popup_configured.push(false);
    surface.commit();
    Ok(Popup {
        surface,
        xdg_surface,
        popup,
        viewport,
        blur,
        target: ShmTarget::default(),
        shown: None,
    })
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
        _: &PopupLevel,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_popup::Event::PopupDone = event {
            state.popup_dismissed = true;
        }
    }
}

delegate_noop!(State: ignore XdgPositioner);
