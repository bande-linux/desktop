mod layout;
mod paint;

use std::time::Duration;

use bande_spring::{Animated, Spring, State as Phase};
use smithay::backend::renderer::element::Id;
use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::input::keyboard::keysyms;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::shell::wlr_layer::Layer;

use self::layout::lerp;
pub(crate) use self::layout::{arrange, real_rect};
use crate::gesture::Tracker;
use crate::label::Label;
use crate::motion::{Look, Motion, Tick};
use crate::shell::toplevel_names;
use crate::state::{Compositor, State};
use crate::window::Window;

type Frame = Rectangle<f64, Logical>;

const SWIPE_FOR_OVERVIEW: f64 = 300.0;
const DRAG_START: f64 = 4.0;
const DIMMED: f64 = 0.25;
const MOMENTUM_SPEED: f64 = 2.0;
const BACKDROP_IDS: usize = 2;

#[derive(Debug, Clone, PartialEq)]
struct Slot {
    surface: WlSurface,
    space: u64,
    rect: Frame,
}

#[derive(Debug, Clone, PartialEq)]
enum Hit {
    Window(WlSurface),
    Space(usize, usize),
    Add(usize),
    Nothing,
}

const BTN_MIDDLE: u32 = 0x112;

#[derive(Debug, Clone)]
struct Press {
    hit: Hit,
    at: Point<f64, Logical>,
    dragging: bool,
    closes: bool,
}

#[derive(Default)]
pub(crate) struct Overview {
    title: Option<(WlSurface, Label)>,
    progress: f64,
    target: f64,
    settle: Option<Animated<Duration>>,
    now: Option<Duration>,
    tracker: Option<Tracker>,
    opens_up: bool,
    app: Option<String>,
    filter: String,
    selected: Option<WlSurface>,
    hovered: Option<Hit>,
    press: Option<Press>,
    pointer: Point<f64, Logical>,
    returning: Option<(WlSurface, Motion)>,
    slots: Vec<Slot>,
    ids: Vec<Vec<Id>>,
}

impl Overview {
    pub(crate) fn is_open(&self) -> bool {
        self.target != 0.0 || self.tracker.is_some()
    }

    pub(crate) fn is_shown(&self) -> bool {
        self.is_open() || self.progress != 0.0
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.settle.is_some() || self.returning.is_some()
    }

    fn slot(&self, surface: &WlSurface) -> Option<&Slot> {
        self.slots.iter().find(|slot| &slot.surface == surface)
    }

    pub(crate) fn is_pressed(&self) -> bool {
        self.press.is_some()
    }

    fn dragged(&self) -> Option<&WlSurface> {
        match &self.press {
            Some(Press {
                hit: Hit::Window(surface),
                dragging: true,
                ..
            }) => Some(surface),
            _ => None,
        }
    }

    fn settle_to(&mut self, target: f64, velocity: f64, spring: Spring) {
        let velocity = match &self.settle {
            Some(settle) if velocity == 0.0 => {
                settle.state_at(self.now.unwrap_or_default()).velocity
            }
            _ => velocity,
        };
        self.tracker = None;
        self.target = target;
        self.now = None;
        self.settle = Some(Animated::released(
            Phase {
                position: self.progress,
                velocity,
            },
            target,
            spring,
        ));
    }
}

impl State {
    fn matches_filter(&self, surface: &WlSurface) -> bool {
        if self.overview.filter.is_empty() {
            return true;
        }
        let (title, app_id) = toplevel_names(surface);
        let filter = &self.overview.filter;
        title.to_lowercase().contains(filter) || app_id.to_lowercase().contains(filter)
    }

    fn selectable(&self, surface: &WlSurface) -> bool {
        self.overview.slots.iter().any(|slot| {
            &slot.surface == surface
                && slot.space
                    == self.screens[self.screen_of_workspace(slot.space)]
                        .workspaces
                        .active_id()
        }) && self.matches_filter(surface)
    }

    pub(crate) fn in_overview(&self, window: &Window, mut look: Look) -> Look {
        let progress = self.overview.progress;
        let Some(slot) = self.overview.slot(window.surface()) else {
            look.alpha *= 1.0 - progress;
            return look;
        };
        let mut target = Look::shown(slot.rect);
        let screen = self.screen_of_workspace(slot.space);
        self.place_on_workspace(&mut target, self.slot_offset(slot), screen);
        if let Some((surface, motion)) = &self.overview.returning
            && surface == window.surface()
            && let Some(returning) = motion.shown()
        {
            target.rect = returning.rect;
        }
        if self.dragged_in_overview(window)
            && let Some(press) = &self.overview.press
        {
            target.rect.loc += self.overview.pointer - press.at;
        }
        if !self.matches_filter(window.surface()) {
            target.alpha *= DIMMED;
        }
        if self.reduce_motion {
            target.alpha *= look.alpha * progress;
            return target;
        }
        Look {
            rect: lerp(look.rect, target.rect, progress),
            alpha: look.alpha + (target.alpha - look.alpha) * progress,
        }
    }

    pub(crate) fn overview_ghost(&self, window: &Window) -> Option<Look> {
        let progress = self.overview.progress;
        if !self.reduce_motion || progress <= 0.0 || progress >= 1.0 {
            return None;
        }
        self.overview.slot(window.surface())?;
        let mut look = Look::shown(real_rect(window)?);
        self.place_on_workspace(
            &mut look,
            self.workspace_offset(window),
            self.screen_of(window),
        );
        look.alpha *= 1.0 - progress;
        Some(look)
    }

    pub(crate) fn dragged_in_overview(&self, window: &Window) -> bool {
        self.overview.dragged().is_some_and(|dragged| {
            self.window_index(dragged)
                .zip(self.window_index(window.surface()))
                .is_some_and(|(dragged, index)| self.descends_from(index, dragged))
        })
    }

    fn overview_spring(&self, velocity: f64) -> Spring {
        if self.reduce_motion {
            Spring::FADE
        } else if velocity.abs() > MOMENTUM_SPEED {
            Spring::MOMENTUM
        } else {
            Spring::STANDARD
        }
    }

    fn prepare_overview(&mut self, app: Option<String>) {
        self.close_menus();
        self.hide_desktop();
        self.overview.app = app;
        self.overview.filter.clear();
        self.overview.press = None;
        self.overview.hovered = None;
        self.overview.returning = None;
        self.overview.selected = self.key_window();
        self.overview.pointer = self.pointer_location;
    }

    pub(crate) fn is_above_windows(&self, surface: &WlSurface) -> bool {
        self.screens.iter().any(|screen| {
            layer_map_for_output(&screen.output)
                .layer_for_surface(surface, WindowSurfaceType::ALL)
                .is_some_and(|layer| matches!(layer.layer(), Layer::Top | Layer::Overlay))
        })
    }

    pub(crate) fn open_overview(&mut self, app: Option<String>) {
        if !self.overview.is_open() {
            self.prepare_overview(app);
        }
        self.settle_overview(1.0, 0.0);
        self.relayout_overview();
    }

    pub(crate) fn close_overview(&mut self) {
        if self.overview.is_open() {
            self.settle_overview(0.0, 0.0);
        }
    }

    fn settle_overview(&mut self, target: f64, velocity: f64) {
        let spring = self.overview_spring(velocity);
        self.overview.settle_to(target, velocity, spring);
        if target == 0.0 {
            self.overview.press = None;
            let keeps_focus = self
                .key_window()
                .and_then(|surface| self.window_index(&surface))
                .is_some_and(|index| self.present(&self.windows[index]));
            if !keeps_focus {
                self.focus_front_of_workspace(self.pointer_screen());
            }
        }
        self.needs_redraw = true;
        self.refocus_pointer();
    }

    pub(crate) fn toggle_overview(&mut self, app: Option<String>) {
        if self.overview.is_open() && self.overview.app == app {
            self.close_overview();
        } else {
            self.overview.app = app.clone();
            self.open_overview(app);
        }
    }

    pub(crate) fn show_app_windows(&mut self) {
        let app = self
            .key_window()
            .map(|surface| toplevel_names(&surface).1)
            .filter(|app_id| !app_id.is_empty());
        if app.is_some() {
            self.toggle_overview(app);
        }
    }

    fn pick(&mut self, surface: WlSurface) {
        self.close_overview();
        self.focus(Some(surface));
    }

    pub(crate) fn animate_overview(&mut self, tick: Tick) {
        let precision = self.progress_precision();
        if let Some((_, motion)) = &mut self.overview.returning {
            motion.advance(tick);
            if motion.is_settled() {
                self.overview.returning = None;
            }
        }
        if let Some(settle) = self.overview.settle.as_mut() {
            let now = tick.drive(settle);
            self.overview.now = Some(now);
            self.overview.progress = settle.state_at(now).position;
            if settle.is_at_rest(now, precision) {
                self.overview.progress = self.overview.target;
                self.overview.settle = None;
                self.overview.now = None;
                if self.overview.target == 0.0 {
                    self.overview.app = None;
                    self.overview.filter.clear();
                    self.tidy_workspaces();
                    self.refocus_pointer();
                }
            }
            self.needs_redraw = true;
        }
        self.relayout_overview();
    }

    pub(crate) fn begin_overview_swipe(&mut self, time: u32, upward: bool) {
        if !self.overview.is_open() {
            let app = if upward {
                None
            } else {
                let app = self
                    .key_window()
                    .map(|surface| toplevel_names(&surface).1)
                    .filter(|app_id| !app_id.is_empty());
                if app.is_none() {
                    return;
                }
                app
            };
            self.prepare_overview(app);
            self.overview.opens_up = upward;
        }
        self.overview.settle = None;
        self.overview.now = None;
        self.overview.tracker = Some(Tracker::new(self.overview.progress, time, 1.0));
        self.relayout_overview();
        self.refocus_pointer();
    }

    pub(crate) fn track_overview_swipe(&mut self, dy: f64, time: u32) {
        let opens_up = self.overview.opens_up;
        let Some(tracker) = self.overview.tracker.as_mut() else {
            return;
        };
        let opening = if opens_up { -dy } else { dy };
        let raw = tracker.raw() + opening / SWIPE_FOR_OVERVIEW;
        tracker.track(raw, time);
        self.overview.progress = tracker.position();
        self.needs_redraw = true;
    }

    pub(crate) fn end_overview_swipe(&mut self, cancelled: bool) {
        let Some(tracker) = self.overview.tracker.take() else {
            return;
        };
        let (target, velocity) = if cancelled {
            (tracker.start().round(), 0.0)
        } else {
            tracker.release()
        };
        self.settle_overview(target, velocity);
    }

    fn hit(&self, at: Point<f64, Logical>) -> Hit {
        if self.overview.app.is_none()
            && let Some(screen) = self.screen_at(at)
        {
            let (thumbnails, add) = self.strip(screen);
            if let Some(index) = thumbnails.iter().position(|rect| rect.contains(at)) {
                return Hit::Space(screen, index);
            }
            if add.contains(at) {
                return Hit::Add(screen);
            }
        }
        let dragged = self.overview.dragged();
        self.overview
            .slots
            .iter()
            .rev()
            .filter(|slot| Some(&slot.surface) != dragged)
            .find(|slot| self.slot_on_screen(slot).contains(at))
            .map_or(Hit::Nothing, |slot| Hit::Window(slot.surface.clone()))
    }

    pub(crate) fn overview_pointer(&mut self, at: Point<f64, Logical>) {
        self.overview.pointer = at;
        if let Some(press) = &mut self.overview.press
            && matches!(press.hit, Hit::Window(_))
            && !press.dragging
            && !press.closes
            && (at - press.at).x.hypot((at - press.at).y) > DRAG_START
        {
            press.dragging = true;
        }
        let hit = self.hit(at);
        if self.overview.press.is_none()
            && let Hit::Window(surface) = &hit
            && self.selectable(surface)
        {
            self.overview.selected = Some(surface.clone());
        }
        self.overview.hovered = Some(hit);
        self.needs_redraw = true;
    }

    pub(crate) fn overview_button(&mut self, button: u32, pressed: bool) {
        let at = self.overview.pointer;
        if pressed {
            self.overview.press = Some(Press {
                hit: self.hit(at),
                at,
                dragging: false,
                closes: button == BTN_MIDDLE,
            });
            return;
        }
        let Some(press) = self.overview.press.take() else {
            return;
        };
        let released = self.hit(at);
        match (press.hit, press.dragging) {
            (Hit::Window(surface), _) if press.closes => {
                if released == Hit::Window(surface.clone())
                    && let Some(index) = self.window_index(&surface)
                {
                    self.windows[index].toplevel.send_close();
                }
            }
            (Hit::Window(surface), true) => self.drop_in_overview(surface, released, at, press.at),
            (Hit::Window(surface), false) if released == Hit::Window(surface.clone()) => {
                self.pick(surface);
            }
            (Hit::Space(screen, index), _) if released == Hit::Space(screen, index) => {
                self.close_overview();
                self.switch_workspace(screen, index);
            }
            (Hit::Add(screen), _) if released == Hit::Add(screen) => {
                self.add_workspace(screen);
                self.relayout_overview();
            }
            (Hit::Nothing, _) if released == Hit::Nothing => self.close_overview(),
            _ => {}
        }
        self.needs_redraw = true;
    }

    fn drop_in_overview(
        &mut self,
        surface: WlSurface,
        target: Hit,
        at: Point<f64, Logical>,
        grabbed: Point<f64, Logical>,
    ) {
        let dropped_on = self.screen_at(at);
        let home = self
            .window_index(&surface)
            .map(|index| self.screen_of(&self.windows[index]));
        match target {
            Hit::Space(screen, index) => self.move_to_space_at(&surface, screen, index),
            Hit::Add(screen) => {
                let count = self.screens[screen].workspaces.count();
                self.move_to_space_at(&surface, screen, count);
            }
            _ if dropped_on.is_some() && dropped_on != home => {
                if let Some(screen) = dropped_on {
                    let active = self.screens[screen].workspaces.active();
                    self.move_to_space_at(&surface, screen, active);
                }
            }
            _ => {
                let Some(slot) = self.overview.slot(&surface).cloned() else {
                    return;
                };
                let home = self.slot_on_screen(&slot);
                let dropped = Rectangle::new(home.loc + (at - grabbed), home.size);
                let spring = if self.reduce_motion {
                    Spring::FADE
                } else {
                    Spring::STANDARD
                };
                let mut motion = Motion::new(spring, Look::shown(dropped), Look::shown(home));
                if self.reduce_motion {
                    motion.place(home);
                }
                self.overview.returning = Some((surface, motion));
                return;
            }
        }
        self.relayout_overview();
    }

    pub(crate) fn overview_key(&mut self, keysym: u32, text: Option<char>) {
        match keysym {
            keysyms::KEY_Escape if !self.overview.filter.is_empty() => {
                self.overview.filter.clear();
            }
            keysyms::KEY_Escape => self.close_overview(),
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => match self.overview.selected.clone() {
                Some(surface) if self.selectable(&surface) => self.pick(surface),
                _ => self.close_overview(),
            },
            keysyms::KEY_Left => self.move_selection((-1.0, 0.0)),
            keysyms::KEY_Right => self.move_selection((1.0, 0.0)),
            keysyms::KEY_Up => self.move_selection((0.0, -1.0)),
            keysyms::KEY_Down => self.move_selection((0.0, 1.0)),
            keysyms::KEY_BackSpace => {
                self.overview.filter.pop();
            }
            _ => {
                if let Some(text) = text.filter(|text| !text.is_control()) {
                    self.overview.filter.extend(text.to_lowercase());
                }
            }
        }
        self.overview.selected = self
            .overview
            .selected
            .clone()
            .filter(|surface| self.selectable(surface));
        self.relayout_overview();
        self.needs_redraw = true;
    }

    fn move_selection(&mut self, (dx, dy): (f64, f64)) {
        let center = |rect: Frame| rect.loc + rect.size.downscale(2.0).to_point();
        let from = self
            .overview
            .selected
            .as_ref()
            .and_then(|surface| self.overview.slot(surface))
            .map(|slot| center(self.slot_on_screen(slot)));
        let Some(from) = from else {
            self.relayout_overview();
            return;
        };
        let next = self
            .overview
            .slots
            .iter()
            .filter(|slot| self.selectable(&slot.surface))
            .filter_map(|slot| {
                let offset = center(self.slot_on_screen(slot)) - from;
                let along = offset.x * dx + offset.y * dy;
                let across = (offset.x * dy - offset.y * dx).abs();
                (along > 0.0).then(|| (along + 2.0 * across, slot.surface.clone()))
            })
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
            .map(|(_, surface)| surface);
        if next.is_some() {
            self.overview.selected = next;
        }
    }
}

impl Compositor {}
