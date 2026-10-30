use std::time::Duration;

use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::desktop::{WindowSurfaceType, layer_map_for_output};
use smithay::input::keyboard::keysyms;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::shell::wlr_layer::Layer;
use unbound_render::VulkanRenderer;
use unbound_spring::{Animation, Spring, State as Phase};

use crate::gesture::Tracker;
use crate::label::{Label, Labels};
use crate::motion::{Look, Motion};
use crate::scene::{CORNER_RADIUS, FillElement, Piece, fresh, pieces_of, rounded, scaled_pieces};
use crate::shell::toplevel_names;
use crate::state::{Headless, OutputElement, State};
use crate::window::Window;

type Frame = Rectangle<f64, Logical>;

const SWIPE_FOR_OVERVIEW: f64 = 300.0;
const MARGIN: f64 = 32.0;
const GAP: f64 = 24.0;
const STRIP_SHARE: f64 = 0.12;
const STRIP_PAD: f64 = 14.0;
const THUMB_GAP: f64 = 16.0;
const ADD_SHARE: f64 = 0.4;
const PLUS_ARM: f64 = 0.3;
const PLUS_THICKNESS: f64 = 2.0;
const DRAG_START: f64 = 4.0;
const TITLE_GAP: f64 = 8.0;
const TITLE_MIN_WIDTH: f64 = 160.0;
const TITLE_ROOM: f64 = 24.0;
const RING: f64 = 4.0;
const DIMMED: f64 = 0.25;
const MOMENTUM_SPEED: f64 = 2.0;
const SETTLED_PROGRESS: f64 = 1e-3;
const SETTLED_SPEED: f64 = 1e-2;
const THUMB_FILL: [f32; 4] = [0.07, 0.08, 0.1, 1.0];
const ADD_FILL: [f32; 4] = [0.12, 0.12, 0.12, 0.12];
const PLUS_FILL: [f32; 4] = [0.75, 0.75, 0.75, 0.75];
const RING_FILL: [f32; 4] = [0.55, 0.55, 0.55, 0.55];
const TARGET_FILL: [f32; 4] = [0.3, 0.3, 0.3, 0.3];

#[derive(Debug, Clone, PartialEq)]
struct Slot {
    surface: WlSurface,
    space: u64,
    rect: Frame,
}

#[derive(Debug, Clone, PartialEq)]
enum Hit {
    Window(WlSurface),
    Space(usize),
    Add,
    Nothing,
}

#[derive(Debug, Clone)]
struct Press {
    hit: Hit,
    at: Point<f64, Logical>,
    dragging: bool,
}

#[derive(Default)]
pub(crate) struct Overview {
    title: Option<(WlSurface, Label)>,
    labels: Option<Labels>,
    progress: f64,
    target: f64,
    settle: Option<Animation>,
    last: Option<Duration>,
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
    ids: Vec<Id>,
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
        self.tracker = None;
        self.target = target;
        self.last = None;
        self.settle = Some(Animation::new(
            spring,
            Phase {
                position: self.progress,
                velocity,
            },
            target,
        ));
    }
}

fn arrange(windows: &[Frame], area: Frame) -> Vec<Frame> {
    if windows.is_empty() || area.size.w <= 0.0 || area.size.h <= 0.0 {
        return vec![area; windows.len()];
    }
    let center = |rect: &Frame| rect.loc + rect.size.downscale(2.0).to_point();
    let mut by_height: Vec<usize> = (0..windows.len()).collect();
    by_height.sort_by(|a, b| {
        let (a, b) = (center(&windows[*a]), center(&windows[*b]));
        a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x))
    });
    let mut best: Option<(f64, Vec<Vec<usize>>)> = None;
    for count in 1..=windows.len() {
        let per_row = windows.len().div_ceil(count);
        let rows: Vec<Vec<usize>> = by_height
            .chunks(per_row)
            .map(|row| {
                let mut row = row.to_vec();
                row.sort_by(|a, b| center(&windows[*a]).x.total_cmp(&center(&windows[*b]).x));
                row
            })
            .collect();
        let heights: f64 = rows.iter().map(|row| row_height(windows, row)).sum::<f64>();
        let gaps = GAP * (rows.len() - 1) as f64;
        let vertical = (area.size.h - gaps).max(1.0) / heights.max(1.0);
        let horizontal = rows
            .iter()
            .map(|row| {
                let widths: f64 = row.iter().map(|index| windows[*index].size.w).sum();
                (area.size.w - GAP * (row.len() - 1) as f64).max(1.0) / widths.max(1.0)
            })
            .fold(f64::INFINITY, f64::min);
        let scale = vertical.min(horizontal).min(1.0);
        if best.as_ref().is_none_or(|(known, _)| scale > *known) {
            best = Some((scale, rows));
        }
    }
    let Some((scale, rows)) = best else {
        return Vec::new();
    };
    let total: f64 = rows
        .iter()
        .map(|row| row_height(windows, row) * scale)
        .sum::<f64>()
        + GAP * (rows.len() - 1) as f64;
    let mut placed = vec![area; windows.len()];
    let mut y = area.loc.y + (area.size.h - total) / 2.0;
    for row in &rows {
        let height = row_height(windows, row) * scale;
        let width: f64 = row
            .iter()
            .map(|index| windows[*index].size.w * scale)
            .sum::<f64>()
            + GAP * (row.len() - 1) as f64;
        let mut x = area.loc.x + (area.size.w - width) / 2.0;
        for index in row {
            let size = windows[*index].size.upscale(scale);
            placed[*index] = Rectangle::new((x, y + (height - size.h) / 2.0).into(), size);
            x += size.w + GAP;
        }
        y += height + GAP;
    }
    placed
}

fn row_height(windows: &[Frame], row: &[usize]) -> f64 {
    row.iter()
        .map(|index| windows[*index].size.h)
        .fold(0.0, f64::max)
}

fn lerp(from: Frame, to: Frame, progress: f64) -> Frame {
    let mix = |a: f64, b: f64| a + (b - a) * progress;
    Rectangle::new(
        (mix(from.loc.x, to.loc.x), mix(from.loc.y, to.loc.y)).into(),
        (mix(from.size.w, to.size.w), mix(from.size.h, to.size.h)).into(),
    )
}

fn inflate(rect: Frame, by: f64) -> Frame {
    Rectangle::new(
        rect.loc - Point::from((by, by)),
        (rect.size.w + 2.0 * by, rect.size.h + 2.0 * by).into(),
    )
}

fn real_rect(window: &Window) -> Option<Frame> {
    let geometry = window.geometry()?;
    Some(Rectangle::new(
        geometry.location.to_f64(),
        geometry.size.to_f64(),
    ))
}

impl State {
    fn thumbnail_scale(&self) -> f64 {
        let output = f64::from(self.logical_output_size().h).max(1.0);
        f64::from(self.usable_area().size.h) * STRIP_SHARE / output
    }

    fn strip(&self) -> (Vec<Frame>, Frame) {
        let scale = self.thumbnail_scale();
        let size = self.logical_output_size().to_f64().upscale(scale);
        let area = self.usable_area();
        let count = self.workspaces.count() as f64;
        let add = Rectangle::new(Point::default(), (size.w * ADD_SHARE, size.h).into());
        let total = count * (size.w + THUMB_GAP) + add.size.w;
        let left = f64::from(area.location.x) + (f64::from(area.size.w) - total) / 2.0;
        let top = f64::from(area.location.y) + STRIP_PAD;
        let thumbnails = (0..self.workspaces.count())
            .map(|index| {
                Rectangle::new(
                    (left + index as f64 * (size.w + THUMB_GAP), top).into(),
                    size,
                )
            })
            .collect();
        let add = Rectangle::new((left + count * (size.w + THUMB_GAP), top).into(), add.size);
        (thumbnails, add)
    }

    fn layout_area(&self) -> Frame {
        let area = self.usable_area();
        let top = if self.overview.app.is_some() {
            f64::from(area.location.y)
        } else {
            let (thumbnails, _) = self.strip();
            thumbnails
                .first()
                .map_or(f64::from(area.location.y), |thumbnail| {
                    thumbnail.loc.y + thumbnail.size.h + STRIP_PAD
                })
        };
        let bottom = f64::from(area.location.y + area.size.h);
        Rectangle::new(
            (f64::from(area.location.x) + MARGIN, top + MARGIN).into(),
            (
                f64::from(area.size.w) - 2.0 * MARGIN,
                bottom - top - 2.0 * MARGIN - TITLE_ROOM,
            )
                .into(),
        )
    }

    fn arranged(&self, windows: Vec<&Window>, space: u64, area: Frame) -> Vec<Slot> {
        let rects: Vec<Frame> = windows
            .iter()
            .filter_map(|window| real_rect(window))
            .collect();
        windows
            .into_iter()
            .zip(arrange(&rects, area))
            .map(|(window, rect)| Slot {
                surface: window.surface().clone(),
                space,
                rect,
            })
            .collect()
    }

    pub(crate) fn relayout_overview(&mut self) {
        if !self.overview.is_shown() {
            self.overview.slots.clear();
            return;
        }
        let area = self.layout_area();
        let shown = |window: &&Window| window.visible() && real_rect(window).is_some();
        let slots = match &self.overview.app {
            Some(app_id) => {
                let windows = self
                    .windows
                    .iter()
                    .filter(shown)
                    .filter(|window| &toplevel_names(window.surface()).1 == app_id)
                    .collect();
                self.arranged(windows, self.workspaces.active_id(), area)
            }
            None => (0..self.workspaces.count())
                .filter_map(|index| self.workspaces.id_at(index))
                .flat_map(|space| {
                    let windows = self
                        .windows
                        .iter()
                        .filter(shown)
                        .filter(|window| window.workspace == space)
                        .collect();
                    self.arranged(windows, space, area)
                })
                .collect(),
        };
        self.overview.slots = slots;
        let wanted = 2 * self.workspaces.count() + 6;
        if self.overview.ids.len() < wanted {
            self.overview.ids.resize_with(wanted, Id::new);
        }
        let selectable = |surface: &WlSurface| self.selectable(surface);
        if !self.overview.selected.as_ref().is_some_and(selectable) {
            self.overview.selected = self
                .overview
                .slots
                .iter()
                .find(|slot| selectable(&slot.surface))
                .map(|slot| slot.surface.clone());
        }
        self.update_title();
    }

    fn update_title(&mut self) {
        let Some((surface, slot)) = self
            .overview
            .selected
            .clone()
            .filter(|_| self.overview.dragged().is_none())
            .and_then(|surface| {
                let slot = self.overview.slot(&surface)?.rect;
                Some((surface, slot))
            })
        else {
            self.overview.title = None;
            return;
        };
        let (title, app_id) = toplevel_names(&surface);
        let text = if title.is_empty() { app_id } else { title };
        let scale = self.output_scale;
        if self.overview.title.as_ref().is_some_and(|(known, label)| {
            known == &surface && label.text == text && label.scale == scale
        }) {
            return;
        }
        let width = slot.size.w.max(TITLE_MIN_WIDTH);
        let label = self
            .overview
            .labels
            .get_or_insert_with(Labels::default)
            .render(&text, width, scale);
        self.overview.title = label.map(|label| (surface, label));
    }

    fn title_element(&self, renderer: &mut VulkanRenderer, scale: f64) -> Option<OutputElement> {
        let (surface, label) = self.overview.title.as_ref()?;
        let index = self.window_index(surface)?;
        let rect = self.presented(&self.windows[index])?.rect;
        let output = self.logical_output_size().to_f64();
        let x = rect.loc.x + (rect.size.w - label.size.w) / 2.0;
        let y = (rect.loc.y + rect.size.h + TITLE_GAP).min(output.h - label.size.h);
        let location = Point::<f64, Logical>::from((x, y))
            .to_physical(scale)
            .to_i32_round::<i32>()
            .to_f64();
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            &label.buffer,
            Some(self.overview.progress as f32),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
        .map(OutputElement::Memory)
    }

    fn matches_filter(&self, surface: &WlSurface) -> bool {
        if self.overview.filter.is_empty() {
            return true;
        }
        let (title, app_id) = toplevel_names(surface);
        let filter = &self.overview.filter;
        title.to_lowercase().contains(filter) || app_id.to_lowercase().contains(filter)
    }

    fn selectable(&self, surface: &WlSurface) -> bool {
        self.overview
            .slots
            .iter()
            .any(|slot| &slot.surface == surface && slot.space == self.workspaces.active_id())
            && self.matches_filter(surface)
    }

    fn slot_offset(&self, slot: &Slot) -> f64 {
        self.workspaces
            .index_of(slot.space)
            .map_or(0.0, |index| index as f64 - self.workspaces.shown_view())
    }

    fn slot_on_screen(&self, slot: &Slot) -> Frame {
        let mut look = Look::shown(slot.rect);
        if !self.reduce_motion {
            self.place_on_workspace(&mut look, self.slot_offset(slot));
        }
        look.rect
    }

    pub(crate) fn in_overview(&self, window: &Window, mut look: Look) -> Look {
        let progress = self.overview.progress;
        let Some(slot) = self.overview.slot(window.surface()) else {
            look.alpha *= 1.0 - progress;
            return look;
        };
        let mut target = Look::shown(slot.rect);
        self.place_on_workspace(&mut target, self.slot_offset(slot));
        if let Some((surface, motion)) = &self.overview.returning
            && surface == window.surface()
            && let Some(returning) = motion.shown()
        {
            target.rect = returning.rect;
        }
        if self.overview.dragged() == Some(window.surface())
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
        self.place_on_workspace(&mut look, self.workspace_offset(window));
        look.alpha *= 1.0 - progress;
        Some(look)
    }

    pub(crate) fn overview_ring(&self, window: &Window, scale: f64) -> Option<OutputElement> {
        if self.overview.selected.as_ref() != Some(window.surface())
            || self.overview.dragged().is_some()
        {
            return None;
        }
        self.overview.slot(window.surface())?;
        let shown = self.presented(window)?;
        let shrink = (shown.rect.size.w / real_rect(window)?.size.w.max(1.0)).min(1.0);
        let rect = inflate(shown.rect, RING);
        let alpha = (self.overview.progress * shown.alpha) as f32;
        Some(OutputElement::Fill(FillElement::new(
            self.overview.ids.first()?.clone(),
            rounded(
                rect.to_physical(scale),
                (CORNER_RADIUS * shrink + RING) * scale,
            ),
            RING_FILL.map(|channel| channel * alpha),
        )))
    }

    pub(crate) fn dragged_in_overview(&self, window: &Window) -> bool {
        self.overview.dragged() == Some(window.surface())
    }

    fn strip_shift(&self) -> f64 {
        if self.reduce_motion {
            return 0.0;
        }
        let (thumbnails, _) = self.strip();
        let reach = thumbnails
            .first()
            .map_or(0.0, |thumbnail| thumbnail.loc.y + thumbnail.size.h);
        -(1.0 - self.overview.progress) * reach
    }

    pub(crate) fn overview_elements(
        &self,
        renderer: &mut VulkanRenderer,
        scale: f64,
    ) -> Vec<OutputElement> {
        let progress = self.overview.progress;
        if progress <= 0.0 {
            return Vec::new();
        }
        let title = self.title_element(renderer, scale);
        if self.overview.app.is_some() {
            return title.into_iter().collect();
        }
        let alpha = progress as f32;
        let shift = Point::from((0.0, self.strip_shift()));
        let (thumbnails, add) = self.strip();
        let factor = self.thumbnail_scale();
        let output = self.output_rect();
        let output = Rectangle::new(output.location.to_f64(), output.size.to_f64());
        let wallpaper: Vec<Piece> = self
            .placed_layers(Layer::Background)
            .into_iter()
            .flat_map(|(surface, at)| {
                let surfaces: Vec<WaylandSurfaceRenderElement<VulkanRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        at.to_f64().to_physical(scale).to_i32_round::<i32>(),
                        scale,
                        1.0,
                        Kind::Unspecified,
                    );
                pieces_of(&surfaces, scale)
            })
            .collect();
        let target = match (&self.overview.press, &self.overview.hovered) {
            (Some(Press { dragging: true, .. }), Some(hovered)) => Some(hovered),
            _ => None,
        };
        let ids = &self.overview.ids;
        let id = |index: usize| ids.get(index).cloned().unwrap_or_else(Id::new);
        let mut elements: Vec<OutputElement> = title.into_iter().collect();
        let plus = Rectangle::new(add.loc + shift, add.size);
        let center = plus.loc + plus.size.downscale(2.0).to_point();
        let arm = plus.size.w.min(plus.size.h) * PLUS_ARM;
        for (index, bar) in [
            Rectangle::new(
                center - Point::from((arm, PLUS_THICKNESS / 2.0)),
                (2.0 * arm, PLUS_THICKNESS).into(),
            ),
            Rectangle::new(
                center - Point::from((PLUS_THICKNESS / 2.0, arm)),
                (PLUS_THICKNESS, 2.0 * arm).into(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            elements.push(OutputElement::Fill(FillElement::new(
                id(1 + index),
                rounded(bar.to_physical(scale), PLUS_THICKNESS / 2.0 * scale),
                PLUS_FILL.map(|channel| channel * alpha),
            )));
        }
        let add_fill = if target == Some(&Hit::Add) {
            TARGET_FILL
        } else {
            ADD_FILL
        };
        elements.push(OutputElement::Fill(FillElement::new(
            id(3),
            rounded(plus.to_physical(scale), CORNER_RADIUS * scale),
            add_fill.map(|channel| channel * alpha),
        )));
        for (index, thumbnail) in thumbnails.iter().enumerate() {
            let thumbnail = Rectangle::new(thumbnail.loc + shift, thumbnail.size);
            let shown = thumbnail.to_physical(scale);
            let radius = CORNER_RADIUS * factor * scale;
            let map = |rect: Frame| {
                Rectangle::new(
                    thumbnail.loc + rect.loc.upscale(factor),
                    rect.size.upscale(factor),
                )
            };
            let space = self.workspaces.id_at(index);
            for window in self
                .windows
                .iter()
                .rev()
                .filter(|window| window.visible() && Some(window.workspace) == space)
            {
                let Some(rect) = real_rect(window) else {
                    continue;
                };
                let surfaces: Vec<WaylandSurfaceRenderElement<VulkanRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        window.surface(),
                        window
                            .surface_origin()
                            .to_f64()
                            .to_physical(scale)
                            .to_i32_round::<i32>(),
                        scale,
                        1.0,
                        Kind::Unspecified,
                    );
                let small = map(rect).to_physical(scale);
                elements.extend(scaled_pieces(
                    &fresh(pieces_of(&surfaces, scale)),
                    rect.to_physical(scale),
                    small,
                    rounded(small, CORNER_RADIUS * factor * scale),
                    alpha,
                ));
            }
            elements.extend(scaled_pieces(
                &fresh(wallpaper.clone()),
                output.to_physical(scale),
                shown,
                rounded(shown, radius),
                alpha,
            ));
            elements.push(OutputElement::Fill(FillElement::new(
                id(4 + 2 * index),
                rounded(shown, radius),
                THUMB_FILL.map(|channel| channel * alpha),
            )));
            let ring = if target == Some(&Hit::Space(index)) {
                Some(TARGET_FILL)
            } else if index == self.workspaces.active() {
                Some(RING_FILL)
            } else {
                None
            };
            if let Some(ring) = ring {
                elements.push(OutputElement::Fill(FillElement::new(
                    id(5 + 2 * index),
                    rounded(
                        inflate(thumbnail, RING / 2.0).to_physical(scale),
                        radius + RING / 2.0 * scale,
                    ),
                    ring.map(|channel| channel * alpha),
                )));
            }
        }
        elements
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
        layer_map_for_output(&self.output)
            .layer_for_surface(surface, WindowSurfaceType::ALL)
            .is_some_and(|layer| matches!(layer.layer(), Layer::Top | Layer::Overlay))
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
                self.focus_front_of_workspace();
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

    pub(crate) fn animate_overview(&mut self, time: Duration) {
        let refresh = self.refresh;
        if let Some((_, motion)) = &mut self.overview.returning {
            motion.advance(time, refresh);
            if motion.is_settled() {
                self.overview.returning = None;
            }
        }
        if let Some(settle) = self.overview.settle.as_mut() {
            let step = self
                .overview
                .last
                .map_or(refresh, |last| time.saturating_sub(last));
            self.overview.last = Some(time);
            self.overview.progress = settle.advance(step.as_secs_f64()).position;
            if settle.is_settled(SETTLED_PROGRESS, SETTLED_SPEED) {
                self.overview.progress = self.overview.target;
                self.overview.settle = None;
                self.overview.last = None;
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
        self.overview.last = None;
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
        if self.overview.app.is_none() {
            let (thumbnails, add) = self.strip();
            if let Some(index) = thumbnails.iter().position(|rect| rect.contains(at)) {
                return Hit::Space(index);
            }
            if add.contains(at) {
                return Hit::Add;
            }
        }
        let dragged = self.overview.dragged();
        self.overview
            .slots
            .iter()
            .filter(|slot| Some(&slot.surface) != dragged)
            .find(|slot| self.slot_on_screen(slot).contains(at))
            .map_or(Hit::Nothing, |slot| Hit::Window(slot.surface.clone()))
    }

    pub(crate) fn overview_pointer(&mut self, at: Point<f64, Logical>) {
        self.overview.pointer = at;
        if let Some(press) = &mut self.overview.press
            && matches!(press.hit, Hit::Window(_))
            && !press.dragging
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

    pub(crate) fn overview_button(&mut self, pressed: bool) {
        let at = self.overview.pointer;
        if pressed {
            self.overview.press = Some(Press {
                hit: self.hit(at),
                at,
                dragging: false,
            });
            return;
        }
        let Some(press) = self.overview.press.take() else {
            return;
        };
        let released = self.hit(at);
        match (press.hit, press.dragging) {
            (Hit::Window(surface), true) => self.drop_in_overview(surface, released, at, press.at),
            (Hit::Window(surface), false) if released == Hit::Window(surface.clone()) => {
                self.pick(surface);
            }
            (Hit::Space(index), _) if released == Hit::Space(index) => {
                self.close_overview();
                self.switch_workspace(index);
            }
            (Hit::Add, _) if released == Hit::Add => {
                self.add_workspace();
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
        match target {
            Hit::Space(index) => self.move_to_space_at(&surface, index),
            Hit::Add => self.move_to_space_at(&surface, self.workspaces.count()),
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

impl Headless {
    pub fn overview_shown(&self) -> bool {
        self.state.overview.is_open()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: f64, y: f64, w: f64, h: f64) -> Frame {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn arranged_windows_fit_the_area_without_overlapping_and_keep_their_left_to_right_order() {
        let windows = [
            frame(600.0, 50.0, 800.0, 600.0),
            frame(0.0, 0.0, 800.0, 600.0),
            frame(300.0, 400.0, 1200.0, 700.0),
            frame(1000.0, 500.0, 400.0, 300.0),
        ];
        let area = frame(32.0, 200.0, 1856.0, 800.0);

        let placed = arrange(&windows, area);

        for (index, rect) in placed.iter().enumerate() {
            assert!(
                rect.loc.x >= area.loc.x - 1e-6
                    && rect.loc.y >= area.loc.y - 1e-6
                    && rect.loc.x + rect.size.w <= area.loc.x + area.size.w + 1e-6
                    && rect.loc.y + rect.size.h <= area.loc.y + area.size.h + 1e-6,
                "{index} {rect:?}"
            );
            let ratio = rect.size.w / rect.size.h;
            let original = windows[index].size.w / windows[index].size.h;
            assert!((ratio - original).abs() < 1e-6);
            for other in &placed[index + 1..] {
                assert!(!rect.overlaps(*other), "{rect:?} {other:?}");
            }
        }
        assert!(placed[1].loc.x < placed[0].loc.x);
    }

    #[test]
    fn a_small_window_is_never_enlarged() {
        let placed = arrange(
            &[frame(10.0, 10.0, 200.0, 100.0)],
            frame(0.0, 0.0, 1000.0, 800.0),
        );

        assert_eq!(placed, [frame(400.0, 350.0, 200.0, 100.0)]);
    }
}
