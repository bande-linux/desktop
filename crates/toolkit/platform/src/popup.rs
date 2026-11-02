use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Instant;

use bande_graphics::{Appearance, CpuRenderer, Event, Palette, Point, Rect, Scene, TextSystem};
use bande_widgets::{
    Animated, FrameTime, MENU_SHADOW_MARGIN, Menu, MenuOutcome, MenuPanel, MenuTracker, Spring,
};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::xdg::shell::client::xdg_popup::{self, XdgPopup};
use wayland_protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity, XdgPositioner,
};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::ZwlrLayerSurfaceV1;

use crate::window::{ShmTarget, State, Target, WindowError};

const SUBMENU_OVERLAP: f32 = 2.0;
const SUBMENU_LIFT: i32 = -4;
const BELOW_GAP: f32 = 4.0;
const SLIDE: f32 = 3.0;
const OPAQUE: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuPlacement {
    AtPoint(Point),
    Below(Rect),
}

#[derive(Clone)]
pub(crate) enum Parent {
    Toplevel(XdgSurface),
    Layer(ZwlrLayerSurfaceV1),
    Popup(XdgSurface),
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
                adjustment: fitting | ConstraintAdjustment::FlipX,
                ..Self::fixed(Rect::new(point.x, point.y, 1.0, 1.0))
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

    fn fixed(anchor_rect: Rect) -> Self {
        Self {
            anchor_rect,
            anchor: Anchor::TopLeft,
            gravity: Gravity::BottomRight,
            offset: (0, 0),
            adjustment: ConstraintAdjustment::empty(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Signals {
    configured: AtomicBool,
    frame_pending: AtomicBool,
    dismissed: AtomicBool,
    x: AtomicI32,
    y: AtomicI32,
    height: AtomicI32,
    placed: AtomicBool,
}

impl Signals {
    fn configured(&self) -> bool {
        self.configured.load(Ordering::Acquire)
    }

    fn position(&self) -> (i32, i32) {
        (
            self.x.load(Ordering::Acquire),
            self.y.load(Ordering::Acquire),
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Look {
    revision: u32,
    highlighted: Option<usize>,
    alternates: usize,
    show_highlight: bool,
    appearance: Appearance,
    scale: f64,
    scrolled: f32,
    height: f32,
}

#[derive(Clone, Copy, PartialEq)]
struct Shown {
    look: Look,
    alpha: u32,
    shift: (i32, i32),
}

struct Popup {
    surface: WlSurface,
    xdg_surface: XdgSurface,
    popup: XdgPopup,
    viewport: Option<WpViewport>,
    target: ShmTarget,
    signals: Arc<Signals>,
    size: (f32, f32),
    anchor: Rect,
    sideways: bool,
    painted: Option<Look>,
    presented: Option<Shown>,
    toward: Option<Point>,
    opacity: Animated,
    slide: Animated,
}

struct Frame<'a> {
    shm: &'a WlShm,
    handle: &'a QueueHandle<State>,
    time: FrameTime,
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

    fn ready(&self) -> bool {
        self.signals.configured() && !self.signals.frame_pending.load(Ordering::Acquire)
    }

    fn enter(&mut self, time: FrameTime, reduce_motion: bool) {
        if self.toward.is_some() {
            return;
        }
        let start = time.on_screen();
        self.opacity.animate_at(start, false, 1.0, Spring::FADE);
        self.slide
            .animate_at(start, reduce_motion, 0.0, Spring::QUICK);
        let (x, y) = self.signals.position();
        let toward = match self.sideways {
            true => {
                let right = x as f32 + self.size.0 / 2.0 > self.anchor.x + self.anchor.width / 2.0;
                Point::new(if right { -1.0 } else { 1.0 }, 0.0)
            }
            false => {
                let below = y as f32 + self.size.1 / 2.0 > self.anchor.y + self.anchor.height / 2.0;
                Point::new(0.0, if below { -1.0 } else { 1.0 })
            }
        };
        self.toward = Some(toward);
    }

    fn paint(
        &mut self,
        look: Look,
        cpu: &mut CpuRenderer,
        scene: impl FnOnce() -> Scene,
    ) -> Result<(), WindowError> {
        if self.painted == Some(look) {
            return Ok(());
        }
        let scene = scene();
        if let Some(viewport) = &self.viewport {
            viewport.set_destination(scene.width as i32, scene.height as i32);
        }
        self.target.render(cpu, &scene, look.scale, None)?;
        self.painted = Some(look);
        Ok(())
    }

    fn present(&mut self, frame: &Frame<'_>, keep_going: bool) -> Result<(), WindowError> {
        let Some(look) = self.painted else {
            return Ok(());
        };
        let now = frame.time.presentation;
        let toward = self.toward.unwrap_or(Point::new(0.0, 0.0));
        let (opacity, fading) = self.opacity.value_at(now);
        let (slide, sliding) = self.slide.value_at(now);
        let offset = |direction: f32| (direction * slide * look.scale as f32).round() as i32;
        let next = Shown {
            look,
            alpha: (opacity.clamp(0.0, 1.0) * OPAQUE as f32).round() as u32,
            shift: (offset(toward.x), offset(toward.y)),
        };
        let more = fading || sliding || keep_going;
        if more {
            self.signals.frame_pending.store(true, Ordering::Release);
            self.surface.frame(frame.handle, self.signals.clone());
        }
        if self.presented == Some(next) {
            if more {
                self.surface.commit();
            }
            return Ok(());
        }
        self.presented = Some(next);
        self.target.present_moved(
            Target {
                shm: frame.shm,
                handle: frame.handle,
                surface: &self.surface,
            },
            next.alpha,
            next.shift,
        )
    }
}

pub(crate) struct Menus {
    tracker: MenuTracker,
    popups: Vec<Popup>,
    root: Parent,
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
        parent: Parent,
        placement: MenuPlacement,
        opacity: f32,
    ) -> Result<Self, WindowError> {
        let panel = MenuPanel::new(menu, opening.text, &opening.palette);
        let size = panel.size();
        let mut popup = create(
            opening.state,
            opening.handle,
            &parent,
            Placement::menu(placement),
            (size.width, size.height),
            Some(0),
        )?;
        popup.opacity = Animated::new(opacity);
        popup.slide = Animated::points(SLIDE * (1.0 - opacity));
        Ok(Self {
            tracker: MenuTracker::new(panel, opening.state.input.keyboard_initiated),
            popups: vec![popup],
            root: parent,
        })
    }

    pub(crate) fn waiting(&self) -> bool {
        self.popups
            .first()
            .is_some_and(|popup| !popup.signals.configured())
            && !self.dismissed()
    }

    pub(crate) fn dismissed(&self) -> bool {
        self.popups
            .iter()
            .any(|popup| popup.signals.dismissed.load(Ordering::Acquire))
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.tracker.deadline()
    }

    pub(crate) fn tracker(&self) -> &MenuTracker {
        &self.tracker
    }

    pub(crate) fn handle(&mut self, level: usize, event: &Event, now: Instant) -> MenuOutcome {
        let local =
            |point: Point| Point::new(point.x - MENU_SHADOW_MARGIN, point.y - MENU_SHADOW_MARGIN);
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
            Event::ModifiersChanged(modifiers) => self.tracker.show_alternates(*modifiers),
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
            &Parent::Popup(parent.xdg_surface.clone()),
            Placement::submenu(self.tracker.panels()[level].row_rect(index)),
            (size.width, size.height),
            Some(level + 1),
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
        if let Some(popup) = self.popups.first_mut() {
            popup.size = (size.width, size.height);
            set_shape(
                compositor,
                opening.handle,
                &popup.surface,
                &popup.xdg_surface,
                popup.size,
                true,
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

    pub(crate) fn destroy(self, state: &mut State) {
        state.forget_popups();
        for popup in self.popups.into_iter().rev() {
            popup.destroy();
        }
    }

    pub(crate) fn close(
        self,
        state: &mut State,
        handle: &QueueHandle<State>,
        now: Instant,
    ) -> Fading {
        state.forget_popups();
        let mut ghosts = Vec::new();
        let mut origin = (0, 0);
        for popup in &self.popups {
            if popup.presented.is_none() {
                break;
            }
            let (x, y) = popup.signals.position();
            origin = (origin.0 + x, origin.1 + y);
            let placement = Placement::fixed(Rect::new(origin.0 as f32, origin.1 as f32, 1.0, 1.0));
            match create(state, handle, &self.root, placement, popup.size, None) {
                Ok(ghost) => ghosts.push(ghost),
                Err(error) => {
                    tracing::warn!(%error, "closing the menu without a fade");
                    break;
                }
            }
        }
        let mut originals = self.popups;
        for (ghost, original) in ghosts.iter_mut().zip(&mut originals) {
            ghost.target = std::mem::take(&mut original.target);
            ghost.painted = original.painted;
            ghost.toward = original.toward;
            ghost.slide = original.slide;
            ghost.opacity = original.opacity;
            ghost.opacity.animate_at(now, false, 0.0, Spring::FADE);
        }
        Fading { originals, ghosts }
    }

    fn fit_placements(
        &mut self,
        state: &mut State,
        handle: &QueueHandle<State>,
    ) -> Result<(), WindowError> {
        let compositor = state
            .compositor
            .as_ref()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        for (level, popup) in self.popups.iter_mut().enumerate() {
            if !popup.signals.placed.swap(false, Ordering::AcqRel) {
                continue;
            }
            let height = popup.signals.height.load(Ordering::Acquire);
            let beside_left = popup.signals.x.load(Ordering::Acquire) < 0;
            self.tracker.placed(level, height as f32, beside_left);
            let size = self.tracker.panels()[level].size();
            popup.size = (size.width, size.height);
            set_shape(
                compositor,
                handle,
                &popup.surface,
                &popup.xdg_surface,
                popup.size,
                true,
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
        time: FrameTime,
    ) -> Result<(), WindowError> {
        self.fit_placements(state, handle)?;
        let palette = appearance.palette();
        let shm = state.shm.clone().ok_or(WindowError::Missing("wl_shm"))?;
        let frame = Frame {
            shm: &shm,
            handle,
            time,
        };
        let flashing = self.tracker.is_flashing();
        for (level, (popup, panel)) in self
            .popups
            .iter_mut()
            .zip(self.tracker.panels())
            .enumerate()
        {
            if !popup.ready() {
                continue;
            }
            popup.enter(time, appearance.reduce_motion);
            let show_highlight = self.tracker.shows_highlight(level, time.presentation);
            let look = Look {
                revision: self.tracker.revision(),
                highlighted: panel.highlighted(),
                alternates: panel.shows_alternates(),
                show_highlight,
                appearance,
                scale,
                scrolled: panel.scroll_offset(),
                height: panel.size().height,
            };
            popup.paint(look, cpu, || panel.scene(&palette, show_highlight))?;
            popup.present(&frame, flashing)?;
        }
        Ok(())
    }
}

pub(crate) struct Fading {
    originals: Vec<Popup>,
    ghosts: Vec<Popup>,
}

impl Fading {
    pub(crate) fn opacity(&self, now: Instant) -> f32 {
        match self.originals.is_empty() {
            true => self
                .ghosts
                .first()
                .map_or(0.0, |ghost| ghost.opacity.value_at(now).0.clamp(0.0, 1.0)),
            false => 1.0,
        }
    }

    pub(crate) fn destroy(self) {
        for popup in self
            .ghosts
            .into_iter()
            .chain(self.originals.into_iter().rev())
        {
            popup.destroy();
        }
    }

    pub(crate) fn draw(
        &mut self,
        state: &State,
        handle: &QueueHandle<State>,
        time: FrameTime,
    ) -> Result<bool, WindowError> {
        if self.ghosts.iter().any(|ghost| !ghost.signals.configured()) {
            return Ok(true);
        }
        let shm = state.shm.clone().ok_or(WindowError::Missing("wl_shm"))?;
        let frame = Frame {
            shm: &shm,
            handle,
            time,
        };
        for ghost in &mut self.ghosts {
            if ghost.ready() {
                ghost.present(&frame, false)?;
            }
        }
        for original in self.originals.drain(..).rev() {
            original.destroy();
        }
        Ok(self.ghosts.iter().any(|ghost| {
            ghost.opacity.value_at(time.presentation).1
                || ghost.signals.frame_pending.load(Ordering::Acquire)
        }))
    }
}

fn create(
    state: &mut State,
    handle: &QueueHandle<State>,
    parent: &Parent,
    placement: Placement,
    size: (f32, f32),
    level: Option<usize>,
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
    let signals = Arc::new(Signals::default());
    let surface = compositor.create_surface(handle, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, handle, signals.clone());
    let popup = match parent {
        Parent::Toplevel(parent) | Parent::Popup(parent) => {
            xdg_surface.get_popup(Some(parent), &positioner, handle, signals.clone())
        }
        Parent::Layer(layer) => {
            let popup = xdg_surface.get_popup(None, &positioner, handle, signals.clone());
            layer.get_popup(&popup);
            popup
        }
    };
    positioner.destroy();
    set_shape(
        compositor,
        handle,
        &surface,
        &xdg_surface,
        size,
        level.is_some(),
    );
    if let Some(level) = level {
        if let (Some(seat), Some(serial)) = (&state.seat, state.input.serial) {
            popup.grab(seat, serial);
        }
        state.popup_surfaces.truncate(level);
        state.popup_surfaces.push(surface.clone());
    }
    let viewport = state
        .scaling
        .viewporter
        .as_ref()
        .map(|viewporter| viewporter.get_viewport(&surface, handle, ()));
    surface.commit();
    Ok(Popup {
        surface,
        xdg_surface,
        popup,
        viewport,
        target: ShmTarget::default(),
        signals,
        size,
        anchor: anchor_rect,
        sideways: matches!(parent, Parent::Popup(_)),
        painted: None,
        presented: None,
        toward: None,
        opacity: Animated::new(0.0),
        slide: Animated::points(SLIDE),
    })
}

fn set_shape(
    compositor: &WlCompositor,
    handle: &QueueHandle<State>,
    surface: &WlSurface,
    xdg_surface: &XdgSurface,
    size: (f32, f32),
    takes_input: bool,
) {
    let whole = |value: f32| value.round() as i32;
    let (x, y) = (whole(MENU_SHADOW_MARGIN), whole(MENU_SHADOW_MARGIN));
    let (width, height) = (whole(size.0).max(1), whole(size.1).max(1));
    xdg_surface.set_window_geometry(x, y, width, height);
    let input = compositor.create_region(handle, ());
    if takes_input {
        input.add(x, y, width, height);
    }
    surface.set_input_region(Some(&input));
    input.destroy();
}

impl Dispatch<XdgSurface, Arc<Signals>> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        signals: &Arc<Signals>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            signals.configured.store(true, Ordering::Release);
            state.popups_changed = true;
        }
    }
}

impl Dispatch<XdgPopup, Arc<Signals>> for State {
    fn event(
        state: &mut Self,
        _: &XdgPopup,
        event: xdg_popup::Event,
        signals: &Arc<Signals>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { x, y, height, .. } => {
                signals.x.store(x, Ordering::Release);
                signals.y.store(y, Ordering::Release);
                signals.height.store(height, Ordering::Release);
                signals.placed.store(true, Ordering::Release);
            }
            xdg_popup::Event::PopupDone => {
                signals.dismissed.store(true, Ordering::Release);
                state.popups_changed = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<WlCallback, Arc<Signals>> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        signals: &Arc<Signals>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            signals.frame_pending.store(false, Ordering::Release);
            state.popups_changed = true;
        }
    }
}

delegate_noop!(State: ignore XdgPositioner);
