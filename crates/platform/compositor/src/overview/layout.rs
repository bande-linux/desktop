use smithay::backend::renderer::element::Id;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Point, Rectangle};

use super::{BACKDROP_IDS, Frame, Slot};
use crate::motion::Look;
use crate::shell::toplevel_names;
use crate::state::State;
use crate::window::Window;

const MARGIN: f64 = 32.0;
const GAP: f64 = 24.0;
const STRIP_SHARE: f64 = 0.12;
const STRIP_PAD: f64 = 14.0;
const THUMB_GAP: f64 = 16.0;
const ADD_SHARE: f64 = 0.4;
const TITLE_ROOM: f64 = 24.0;

pub(crate) fn arrange(windows: &[Frame], area: Frame) -> Vec<Frame> {
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

pub(super) fn row_height(windows: &[Frame], row: &[usize]) -> f64 {
    row.iter()
        .map(|index| windows[*index].size.h)
        .fold(0.0, f64::max)
}

pub(super) fn lerp(from: Frame, to: Frame, progress: f64) -> Frame {
    let mix = |a: f64, b: f64| a + (b - a) * progress;
    Rectangle::new(
        (mix(from.loc.x, to.loc.x), mix(from.loc.y, to.loc.y)).into(),
        (mix(from.size.w, to.size.w), mix(from.size.h, to.size.h)).into(),
    )
}

pub(super) fn inflate(rect: Frame, by: f64) -> Frame {
    Rectangle::new(
        rect.loc - Point::from((by, by)),
        (rect.size.w + 2.0 * by, rect.size.h + 2.0 * by).into(),
    )
}

pub(crate) fn real_rect(window: &Window) -> Option<Frame> {
    let geometry = window.geometry()?;
    Some(Rectangle::new(
        geometry.loc.to_f64(),
        geometry.size.to_f64(),
    ))
}

impl State {
    pub(super) fn thumbnail_scale(&self, screen: usize) -> f64 {
        let output = f64::from(self.screen_rect(screen).size.h).max(1.0);
        f64::from(self.usable_area(screen).size.h) * STRIP_SHARE / output
    }

    pub(super) fn strip(&self, screen: usize) -> (Vec<Frame>, Frame) {
        let scale = self.thumbnail_scale(screen);
        let size = self.screen_rect(screen).size.to_f64().upscale(scale);
        let area = self.usable_area(screen);
        let workspaces = &self.screens[screen].workspaces;
        let count = workspaces.count() as f64;
        let add = Rectangle::new(Point::default(), (size.w * ADD_SHARE, size.h).into());
        let total = count * (size.w + THUMB_GAP) + add.size.w;
        let left = f64::from(area.loc.x) + (f64::from(area.size.w) - total) / 2.0;
        let top = f64::from(area.loc.y) + STRIP_PAD;
        let thumbnails = (0..workspaces.count())
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

    fn layout_area(&self, screen: usize) -> Frame {
        let area = self.usable_area(screen);
        let top = if self.overview.app.is_some() {
            f64::from(area.loc.y)
        } else {
            let (thumbnails, _) = self.strip(screen);
            thumbnails
                .first()
                .map_or(f64::from(area.loc.y), |thumbnail| {
                    thumbnail.loc.y + thumbnail.size.h + STRIP_PAD
                })
        };
        let bottom = f64::from(area.loc.y + area.size.h);
        Rectangle::new(
            (f64::from(area.loc.x) + MARGIN, top + MARGIN).into(),
            (
                f64::from(area.size.w) - 2.0 * MARGIN,
                bottom - top - 2.0 * MARGIN - TITLE_ROOM,
            )
                .into(),
        )
    }

    fn family_root(&self, window: &Window) -> Option<&Window> {
        let index = self.window_index(window.surface())?;
        Some(&self.windows[self.root_index(index)])
    }

    fn arranged(&self, windows: Vec<&Window>, space: u64, area: Frame) -> Vec<Slot> {
        let (attached, roots): (Vec<&Window>, Vec<&Window>) = windows.iter().partition(|window| {
            self.family_root(window).is_some_and(|root| {
                root.surface() != window.surface()
                    && windows
                        .iter()
                        .any(|shown| shown.surface() == root.surface())
            })
        });
        let rects: Vec<Frame> = roots
            .iter()
            .filter_map(|window| real_rect(window))
            .collect();
        let mut slots: Vec<Slot> = roots
            .into_iter()
            .zip(arrange(&rects, area))
            .map(|(window, rect)| Slot {
                surface: window.surface().clone(),
                space,
                rect,
            })
            .collect();
        let carried: Vec<Slot> = attached
            .into_iter()
            .filter_map(|window| {
                let root = self.family_root(window)?;
                let slot = slots.iter().find(|slot| &slot.surface == root.surface())?;
                let (home, own) = (real_rect(root)?, real_rect(window)?);
                let scale = slot.rect.size.w / home.size.w.max(1.0);
                Some(Slot {
                    surface: window.surface().clone(),
                    space,
                    rect: Rectangle::new(
                        slot.rect.loc + (own.loc - home.loc).upscale(scale),
                        own.size.upscale(scale),
                    ),
                })
            })
            .collect();
        slots.extend(carried);
        slots
    }

    pub(crate) fn relayout_overview(&mut self) {
        if !self.overview.is_shown() {
            self.overview.slots.clear();
            return;
        }
        let shown = |window: &&Window| window.visible() && real_rect(window).is_some();
        let mut slots = Vec::new();
        for screen in 0..self.screens.len() {
            let area = self.layout_area(screen);
            let workspaces = &self.screens[screen].workspaces;
            let on_screen = |window: &&Window| self.screen_of(window) == screen;
            match &self.overview.app {
                Some(app_id) => {
                    let windows = self
                        .windows
                        .iter()
                        .filter(shown)
                        .filter(on_screen)
                        .filter(|window| &toplevel_names(window.surface()).1 == app_id)
                        .collect();
                    slots.extend(self.arranged(windows, workspaces.active_id(), area));
                }
                None => {
                    for space in workspaces.ids() {
                        let windows = self
                            .windows
                            .iter()
                            .filter(shown)
                            .filter(|window| window.workspace == space)
                            .collect();
                        slots.extend(self.arranged(windows, space, area));
                    }
                }
            }
        }
        self.overview.slots = slots;
        self.overview.ids.resize_with(self.screens.len(), Vec::new);
        for (screen, ids) in self.overview.ids.iter_mut().enumerate() {
            let wanted = 2 * self.screens[screen].workspaces.count() + 4 + BACKDROP_IDS;
            if ids.len() < wanted {
                ids.resize_with(wanted, Id::new);
            }
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

    pub(super) fn slot_offset(&self, slot: &Slot) -> f64 {
        let workspaces = &self.screens[self.screen_of_workspace(slot.space)].workspaces;
        workspaces
            .index_of(slot.space)
            .map_or(0.0, |index| index as f64 - workspaces.shown_view())
    }

    pub(super) fn slot_on_screen(&self, slot: &Slot) -> Frame {
        let mut look = Look::shown(slot.rect);
        if !self.reduce_motion {
            let screen = self.screen_of_workspace(slot.space);
            self.place_on_workspace(&mut look, self.slot_offset(slot), screen);
        }
        look.rect
    }

    pub(super) fn strip_shift(&self, screen: usize) -> f64 {
        if self.reduce_motion {
            return 0.0;
        }
        let (thumbnails, _) = self.strip(screen);
        let reach = thumbnails
            .first()
            .map_or(0.0, |thumbnail| thumbnail.loc.y + thumbnail.size.h);
        -(1.0 - self.overview.progress) * reach
    }
}
