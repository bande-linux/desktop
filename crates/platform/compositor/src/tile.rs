use std::time::Duration;

use bande_protocols::server::bande_app_v1;
use bande_spring::Spring;
use smithay::backend::renderer::element::Id;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};

use crate::motion::{Look, Motion, Tick};
use crate::scene::Casting;
use crate::state::{Compositor, State};

const SHARED_EDGE_HOLD: Duration = Duration::from_millis(250);
const EDGE_REACH: f64 = 1.0;
const CORNER_REACH: i32 = 64;
const WINDOW_TILED_SINCE: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(crate) enum Tile {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Whole,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Snap {
    Tile(Tile),
    Zoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Placing {
    Tile(Tile),
    Zoom,
    Previous,
}

impl Tile {
    pub(crate) fn opposite(self) -> Self {
        Self::on_side(!self.on_left(), self.row())
    }

    fn on_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    fn row(self) -> Row {
        match self {
            Self::Left | Self::Right => Row::Whole,
            Self::TopLeft | Self::TopRight => Row::Top,
            Self::BottomLeft | Self::BottomRight => Row::Bottom,
        }
    }

    fn on_side(left: bool, row: Row) -> Self {
        match (left, row) {
            (true, Row::Whole) => Self::Left,
            (false, Row::Whole) => Self::Right,
            (true, Row::Top) => Self::TopLeft,
            (false, Row::Top) => Self::TopRight,
            (true, Row::Bottom) => Self::BottomLeft,
            (false, Row::Bottom) => Self::BottomRight,
        }
    }

    pub(crate) fn rect(self, area: Rectangle<i32, Logical>) -> Rectangle<i32, Logical> {
        let left_width = area.size.w / 2;
        let top_height = area.size.h / 2;
        let (x, width) = if self.on_left() {
            (area.loc.x, left_width)
        } else {
            (area.loc.x + left_width, area.size.w - left_width)
        };
        let (y, height) = match self.row() {
            Row::Whole => (area.loc.y, area.size.h),
            Row::Top => (area.loc.y, top_height),
            Row::Bottom => (area.loc.y + top_height, area.size.h - top_height),
        };
        Rectangle::new((x, y).into(), (width, height).into())
    }
}

fn right(rect: Rectangle<i32, Logical>) -> i32 {
    rect.loc.x + rect.size.w
}

fn bottom(rect: Rectangle<i32, Logical>) -> i32 {
    rect.loc.y + rect.size.h
}

fn shared_side(
    (tile, ours): (Tile, Rectangle<i32, Logical>),
    (their_tile, theirs): (Tile, Rectangle<i32, Logical>),
) -> Option<Side> {
    let within_height = theirs.loc.y >= ours.loc.y && bottom(theirs) <= bottom(ours);
    let within_width = theirs.loc.x >= ours.loc.x && right(theirs) <= right(ours);
    let same_column = tile.on_left() == their_tile.on_left();
    if tile.on_left() && !their_tile.on_left() && within_height && theirs.loc.x == right(ours) {
        Some(Side::Right)
    } else if !tile.on_left()
        && their_tile.on_left()
        && within_height
        && right(theirs) == ours.loc.x
    {
        Some(Side::Left)
    } else if same_column
        && tile.row() == Row::Top
        && their_tile.row() == Row::Bottom
        && within_width
        && theirs.loc.y == bottom(ours)
    {
        Some(Side::Bottom)
    } else if same_column
        && tile.row() == Row::Bottom
        && their_tile.row() == Row::Top
        && within_width
        && bottom(theirs) == ours.loc.y
    {
        Some(Side::Top)
    } else {
        None
    }
}

fn moved_sides(
    from: Rectangle<i32, Logical>,
    to: Rectangle<i32, Logical>,
) -> impl Iterator<Item = Side> {
    [
        (from.loc.x != to.loc.x, Side::Left),
        (right(from) != right(to), Side::Right),
        (from.loc.y != to.loc.y, Side::Top),
        (bottom(from) != bottom(to), Side::Bottom),
    ]
    .into_iter()
    .filter_map(|(moved, side)| moved.then_some(side))
}

fn placing(tile: Option<Tile>, zoomed: bool, direction: Direction) -> Option<Placing> {
    let row = tile.map(Tile::row);
    let left = tile.map(Tile::on_left);
    match (direction, left, row) {
        (Direction::Left, ..) => Some(Placing::Tile(Tile::Left)),
        (Direction::Right, ..) => Some(Placing::Tile(Tile::Right)),
        (Direction::Up, Some(left), Some(Row::Whole)) => {
            Some(Placing::Tile(Tile::on_side(left, Row::Top)))
        }
        (Direction::Up, Some(left), Some(Row::Bottom)) => {
            Some(Placing::Tile(Tile::on_side(left, Row::Whole)))
        }
        (Direction::Down, Some(left), Some(Row::Whole)) => {
            Some(Placing::Tile(Tile::on_side(left, Row::Bottom)))
        }
        (Direction::Down, Some(left), Some(Row::Top)) => {
            Some(Placing::Tile(Tile::on_side(left, Row::Whole)))
        }
        (Direction::Up, None, _) if !zoomed => Some(Placing::Zoom),
        (Direction::Down, None, _) => Some(Placing::Previous),
        _ => None,
    }
}

pub(crate) fn snap_at(pointer: Point<f64, Logical>, area: Rectangle<i32, Logical>) -> Option<Snap> {
    let left = pointer.x <= f64::from(area.loc.x) + EDGE_REACH;
    let right = pointer.x >= f64::from(area.loc.x + area.size.w) - 1.0 - EDGE_REACH;
    let upper = pointer.y < f64::from(area.loc.y + CORNER_REACH);
    let lower = pointer.y >= f64::from(area.loc.y + area.size.h - CORNER_REACH);
    let row = if upper {
        Row::Top
    } else if lower {
        Row::Bottom
    } else {
        Row::Whole
    };
    if left || right {
        return Some(Snap::Tile(Tile::on_side(left, row)));
    }
    (pointer.y <= f64::from(area.loc.y) + EDGE_REACH).then_some(Snap::Zoom)
}

type Target = (usize, Snap);

pub(crate) struct SnapDrag {
    surface: WlSurface,
    candidate: Option<Target>,
    pointer: Point<f64, Logical>,
    since: Duration,
    hold: Duration,
    shown: Option<Target>,
}

impl SnapDrag {
    pub(crate) fn surface(&self) -> &WlSurface {
        &self.surface
    }
}

pub(crate) struct Preview {
    pub(crate) owner: WlSurface,
    pub(crate) motion: Motion,
    pub(crate) target: Look,
    pub(crate) fill: Id,
    pub(crate) shadow: Casting,
}

impl State {
    fn snap_rect(&self, (screen, snap): Target) -> Rectangle<i32, Logical> {
        let area = self.usable_area(screen);
        match snap {
            Snap::Tile(tile) => tile.rect(area),
            Snap::Zoom => area,
        }
    }

    pub(crate) fn set_tile(&mut self, index: usize, tile: Option<(Tile, Rectangle<i32, Logical>)>) {
        let window = &mut self.windows[index];
        let changed = window.tile.is_some() != tile.is_some();
        window.tile = tile;
        if !changed {
            return;
        }
        let toplevel = window.toplevel.xdg_toplevel().clone();
        for app in self
            .app_resources
            .iter()
            .filter(|app| app.client() == toplevel.client() && app.version() >= WINDOW_TILED_SINCE)
        {
            app.window_tiled(&toplevel, u32::from(tile.is_some()));
        }
    }

    pub(crate) fn tile_window(&mut self, index: usize, tile: Tile, spring: Spring) {
        let target = tile.rect(self.usable_area(self.screen_of(&self.windows[index])));
        self.tile_window_into(index, tile, target, spring);
    }

    pub(crate) fn tile_window_into(
        &mut self,
        index: usize,
        tile: Tile,
        target: Rectangle<i32, Logical>,
        spring: Spring,
    ) {
        if self.windows[index].fixed_size() {
            return;
        }
        let window = &mut self.windows[index];
        if window.restore.is_none()
            && window.tile.is_none()
            && !window.has_state(xdg_toplevel::State::Maximized)
            && !window.has_state(xdg_toplevel::State::Fullscreen)
        {
            window.restore = window.geometry();
        }
        window.toplevel.with_pending_state(|pending| {
            pending.states.unset(xdg_toplevel::State::Maximized);
            pending.states.unset(xdg_toplevel::State::Fullscreen);
        });
        window.request_size(Some(target.size));
        window.move_with_next_configure(target.loc);
        self.set_tile(index, Some((tile, target)));
        self.animate_window(index, spring, None);
    }

    pub(crate) fn untile(&mut self, index: usize) -> Option<Rectangle<i32, Logical>> {
        self.windows[index].tile?;
        self.set_tile(index, None);
        let window = &mut self.windows[index];
        let restore = window.restore.take();
        window.request_size(restore.map(|restore| restore.size));
        match restore {
            Some(restore) => window.move_with_next_configure(restore.loc),
            None => {
                window.configure();
            }
        }
        restore
    }

    pub(crate) fn end_tile_if_reshaped(&mut self, index: usize, from: Rectangle<i32, Logical>) {
        let Some((_, rect)) = self.windows[index].tile else {
            return;
        };
        let shared: Vec<Side> = self
            .tiled_neighbors(index)
            .into_iter()
            .map(|(_, side)| side)
            .collect();
        if moved_sides(from, rect).any(|side| !shared.contains(&side)) {
            self.set_tile(index, None);
            self.windows[index].restore = None;
        }
    }

    pub(crate) fn place_window(
        &mut self,
        toplevel: &xdg_toplevel::XdgToplevel,
        placement: bande_app_v1::Placement,
    ) {
        if let Some(index) = self
            .windows
            .iter()
            .position(|window| window.toplevel.xdg_toplevel() == toplevel)
        {
            self.apply_placement(index, placement);
        }
    }

    pub(crate) fn apply_placement(&mut self, index: usize, placement: bande_app_v1::Placement) {
        if self.windows[index].has_state(xdg_toplevel::State::Fullscreen) {
            return;
        }
        let tile = match placement {
            bande_app_v1::Placement::Left => Tile::Left,
            bande_app_v1::Placement::Right => Tile::Right,
            bande_app_v1::Placement::TopLeft => Tile::TopLeft,
            bande_app_v1::Placement::TopRight => Tile::TopRight,
            bande_app_v1::Placement::BottomLeft => Tile::BottomLeft,
            bande_app_v1::Placement::BottomRight => Tile::BottomRight,
            bande_app_v1::Placement::NextDisplay => {
                let surface = self.windows[index].surface().clone();
                self.move_to_next_screen(&surface);
                return;
            }
            _ => {
                let surface = self.windows[index].surface().clone();
                if self.windows[index].has_state(xdg_toplevel::State::Maximized) {
                    self.unmaximize(&surface);
                } else if self.untile(index).is_some() {
                    self.animate_window(index, Spring::STANDARD, None);
                }
                return;
            }
        };
        self.tile_window(index, tile, Spring::STANDARD);
        self.offer_other_half(index);
    }

    pub(crate) fn place_with_key(&mut self, surface: &WlSurface, direction: Direction) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let window = &self.windows[index];
        if window.has_state(xdg_toplevel::State::Fullscreen) {
            return;
        }
        let tile = window.tile.map(|(tile, _)| tile);
        let zoomed = window.has_state(xdg_toplevel::State::Maximized);
        match placing(tile, zoomed, direction) {
            Some(Placing::Tile(tile)) => {
                self.tile_window(index, tile, Spring::STANDARD);
                self.offer_other_half(index);
            }
            Some(Placing::Zoom) => self.maximize(surface),
            Some(Placing::Previous) if zoomed => self.unmaximize(surface),
            Some(Placing::Previous) | None => {}
        }
    }

    fn tiled_neighbors(&self, index: usize) -> Vec<(usize, Side)> {
        let Some(ours) = self.windows[index].tile else {
            return Vec::new();
        };
        let screen = self.screen_of(&self.windows[index]);
        self.windows
            .iter()
            .enumerate()
            .filter(|(other, window)| {
                *other != index && self.present(window) && self.screen_of(window) == screen
            })
            .filter_map(|(other, window)| Some((other, shared_side(ours, window.tile?)?)))
            .collect()
    }

    pub(crate) fn share_tiled_edge(
        &mut self,
        index: usize,
        mut rect: Rectangle<i32, Logical>,
    ) -> Rectangle<i32, Logical> {
        let Some((tile, old)) = self.windows[index].tile else {
            return rect;
        };
        let neighbors = self.tiled_neighbors(index);
        let dragged: Vec<Side> = moved_sides(old, rect).collect();
        for (neighbor, side) in &neighbors {
            let window = &self.windows[*neighbor];
            let Some((_, theirs)) = window.tile.filter(|_| dragged.contains(side)) else {
                continue;
            };
            let least = window.clamp_size((1, 1).into());
            match side {
                Side::Right => rect.size.w = rect.size.w.min(right(theirs) - least.w - rect.loc.x),
                Side::Bottom => {
                    rect.size.h = rect.size.h.min(bottom(theirs) - least.h - rect.loc.y)
                }
                Side::Left => {
                    let beyond = (theirs.loc.x + least.w - rect.loc.x).max(0);
                    rect.loc.x += beyond;
                    rect.size.w -= beyond;
                }
                Side::Top => {
                    let beyond = (theirs.loc.y + least.h - rect.loc.y).max(0);
                    rect.loc.y += beyond;
                    rect.size.h -= beyond;
                }
            }
        }
        self.windows[index].tile = Some((tile, rect));
        for (neighbor, side) in neighbors {
            let window = &mut self.windows[neighbor];
            let Some((their_tile, theirs)) = window.tile.filter(|_| dragged.contains(&side)) else {
                continue;
            };
            let mut shared = theirs;
            match side {
                Side::Right => {
                    shared.loc.x = right(rect);
                    shared.size.w = right(theirs) - right(rect);
                }
                Side::Left => shared.size.w = rect.loc.x - theirs.loc.x,
                Side::Bottom => {
                    shared.loc.y = bottom(rect);
                    shared.size.h = bottom(theirs) - bottom(rect);
                }
                Side::Top => shared.size.h = rect.loc.y - theirs.loc.y,
            }
            window.tile = Some((their_tile, shared));
            window.request_size(Some(shared.size));
            window.move_with_next_configure(shared.loc);
        }
        rect
    }

    fn borders_another_screen(&self, (screen, snap): Target, pointer: Point<f64, Logical>) -> bool {
        let output = self.screen_rect(screen).to_f64();
        let beyond: Point<f64, Logical> = match snap {
            Snap::Zoom => (pointer.x, output.loc.y - 1.0).into(),
            Snap::Tile(tile) if tile.on_left() => (output.loc.x - 1.0, pointer.y).into(),
            Snap::Tile(_) => (output.loc.x + output.size.w, pointer.y).into(),
        };
        self.screen_at(beyond).is_some()
    }

    pub(crate) fn track_snap(
        &mut self,
        surface: &WlSurface,
        pointer: Point<f64, Logical>,
        time: u32,
    ) {
        let now = Duration::from_millis(u64::from(time));
        let screen = self.pointer_screen();
        let fixed = self
            .window_index(surface)
            .is_some_and(|index| self.windows[index].fixed_size());
        let candidate = snap_at(pointer, self.usable_area(screen))
            .filter(|_| !fixed)
            .map(|snap| (screen, snap));
        let hold = match candidate {
            Some(target) if self.borders_another_screen(target, pointer) => SHARED_EDGE_HOLD,
            _ => Duration::ZERO,
        };
        let drag = self.snap.get_or_insert_with(|| SnapDrag {
            surface: surface.clone(),
            candidate: None,
            pointer,
            since: now,
            hold,
            shown: None,
        });
        drag.pointer = pointer;
        if candidate != drag.candidate {
            drag.candidate = candidate;
            drag.since = now;
            drag.hold = hold;
        }
        if drag.shown.is_some() && drag.shown != candidate {
            drag.shown = None;
            self.hide_preview();
        }
        self.snap_due(now);
    }

    fn snap_due(&mut self, now: Duration) -> Option<Duration> {
        let drag = self.snap.as_mut()?;
        let candidate = drag.candidate?;
        if drag.shown == Some(candidate) {
            return None;
        }
        let due = drag.since + drag.hold;
        if now < due {
            return Some(due);
        }
        drag.shown = Some(candidate);
        let (owner, at) = (drag.surface.clone(), drag.pointer);
        let target = self.snap_rect(candidate);
        let target = Look::shown(target.to_f64());
        let (from, spring) = if self.reduce_motion {
            (Look::hidden(target.rect), Spring::FADE)
        } else {
            (
                Look::hidden(Rectangle::new(at, (1.0, 1.0).into())),
                Spring::FADE,
            )
        };
        let mut motion = match self.preview.take() {
            Some(preview) if preview.owner == owner => preview.motion,
            _ => Motion::new(spring, from, target),
        };
        if self.reduce_motion {
            motion.place(target.rect);
        }
        self.preview = Some(Preview {
            owner,
            motion,
            target,
            fill: Id::new(),
            shadow: Casting::new(),
        });
        self.needs_redraw = true;
        None
    }

    fn hide_preview(&mut self) {
        if let Some(preview) = &mut self.preview {
            preview.target = Look::hidden(preview.target.rect);
            self.needs_redraw = true;
        }
    }

    pub(crate) fn cancel_snap(&mut self) {
        self.snap = None;
        self.hide_preview();
    }

    pub(crate) fn finish_move(&mut self, surface: &WlSurface) {
        let shown = self
            .snap
            .take()
            .filter(|drag| &drag.surface == surface)
            .and_then(|drag| drag.shown);
        self.hide_preview();
        let Some(index) = self.window_index(surface) else {
            return;
        };
        match shown {
            Some((_, Snap::Tile(tile))) => {
                self.tile_window(index, tile, Spring::QUICK);
                self.offer_other_half(index);
            }
            Some((_, Snap::Zoom)) => self.maximize(surface),
            None => {}
        }
    }

    pub(crate) fn animate_preview(&mut self, tick: Tick) {
        let Some(preview) = &mut self.preview else {
            return;
        };
        preview.motion.retarget(preview.target);
        preview.motion.advance(tick);

        if preview.target.alpha == 0.0 && preview.motion.is_settled() {
            self.preview = None;
        }
    }
}

impl Compositor {
    pub fn snap_tick(&mut self, now: Duration) -> Option<Duration> {
        self.state.snap_due(now)
    }
}
