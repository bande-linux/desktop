use std::time::Duration;

use smithay::backend::renderer::element::Id;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use unbound_protocols::server::unbound_app_v1;
use unbound_spring::Spring;

use crate::motion::{Look, Motion};
use crate::scene::Casting;
use crate::state::{Headless, Rect, State};

pub(crate) const REST: Duration = Duration::from_millis(150);
const REST_DISTANCE: f64 = 2.0;
const EDGE_REACH: f64 = 1.0;
const CORNER_REACH: i32 = 64;
const TILED_STATES_SINCE: u32 = 2;
pub(crate) const TILED: [xdg_toplevel::State; 4] = [
    xdg_toplevel::State::TiledLeft,
    xdg_toplevel::State::TiledRight,
    xdg_toplevel::State::TiledTop,
    xdg_toplevel::State::TiledBottom,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    pub(crate) fn rect(self, area: Rect) -> Rect {
        let left_width = area.size.w / 2;
        let top_height = area.size.h / 2;
        let (x, width) = if self.on_left() {
            (area.location.x, left_width)
        } else {
            (area.location.x + left_width, area.size.w - left_width)
        };
        let (y, height) = match self.row() {
            Row::Whole => (area.location.y, area.size.h),
            Row::Top => (area.location.y, top_height),
            Row::Bottom => (area.location.y + top_height, area.size.h - top_height),
        };
        Rect {
            location: (x, y).into(),
            size: (width, height).into(),
        }
    }
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

pub(crate) fn snap_at(pointer: Point<f64, Logical>, output: Rect, area: Rect) -> Option<Snap> {
    let left = pointer.x <= f64::from(output.location.x) + EDGE_REACH;
    let right = pointer.x >= f64::from(output.location.x + output.size.w) - 1.0 - EDGE_REACH;
    let upper = pointer.y < f64::from(area.location.y + CORNER_REACH);
    let lower = pointer.y >= f64::from(area.location.y + area.size.h - CORNER_REACH);
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
    (pointer.y <= f64::from(area.location.y) + EDGE_REACH).then_some(Snap::Zoom)
}

pub(crate) struct SnapDrag {
    surface: WlSurface,
    candidate: Option<Snap>,
    rest_at: Point<f64, Logical>,
    since: Duration,
    shown: Option<Snap>,
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
    fn snap_rect(&self, snap: Snap) -> Rect {
        let area = self.usable_area(self.pointer_screen());
        match snap {
            Snap::Tile(tile) => tile.rect(area),
            Snap::Zoom => area,
        }
    }

    pub(crate) fn tile_window(&mut self, index: usize, tile: Tile) {
        let target = tile.rect(self.usable_area(self.screen_of(&self.windows[index])));
        let window = &mut self.windows[index];
        if window.restore.is_none()
            && window.tile.is_none()
            && !window.has_state(xdg_toplevel::State::Maximized)
            && !window.has_state(xdg_toplevel::State::Fullscreen)
        {
            window.restore = window.geometry();
        }
        let tiled_states = window.toplevel.version() >= TILED_STATES_SINCE;
        window.toplevel.with_pending_state(|pending| {
            pending.states.unset(xdg_toplevel::State::Maximized);
            pending.states.unset(xdg_toplevel::State::Fullscreen);
            if tiled_states {
                for state in TILED {
                    pending.states.set(state);
                }
            }
        });
        window.request_size(Some(target.size));
        window.tile = Some((tile, target));
        if let Some(serial) = window.configure() {
            window.pending_location = Some((serial, target.location));
        }
        self.animate_window(index, Spring::STANDARD, None);
    }

    pub(crate) fn untile(&mut self, index: usize) -> Option<Rect> {
        let window = &mut self.windows[index];
        window.tile.take()?;
        let restore = window.restore.take();
        window.toplevel.with_pending_state(|pending| {
            for state in TILED {
                pending.states.unset(state);
            }
        });
        window.request_size(restore.map(|restore| restore.size));
        let serial = window.configure();
        if let (Some(serial), Some(restore)) = (serial, restore) {
            window.pending_location = Some((serial, restore.location));
        }
        restore
    }

    pub(crate) fn place_window(
        &mut self,
        toplevel: &xdg_toplevel::XdgToplevel,
        placement: unbound_app_v1::Placement,
    ) {
        let Some(index) = self
            .windows
            .iter()
            .position(|window| window.toplevel.xdg_toplevel() == toplevel)
        else {
            return;
        };
        if self.windows[index].has_state(xdg_toplevel::State::Fullscreen) {
            return;
        }
        let tile = match placement {
            unbound_app_v1::Placement::Left => Tile::Left,
            unbound_app_v1::Placement::Right => Tile::Right,
            unbound_app_v1::Placement::TopLeft => Tile::TopLeft,
            unbound_app_v1::Placement::TopRight => Tile::TopRight,
            unbound_app_v1::Placement::BottomLeft => Tile::BottomLeft,
            unbound_app_v1::Placement::BottomRight => Tile::BottomRight,
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
        self.tile_window(index, tile);
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
                self.tile_window(index, tile);
                self.offer_other_half(index);
            }
            Some(Placing::Zoom) => self.maximize(surface),
            Some(Placing::Previous) if zoomed => self.unmaximize(surface),
            Some(Placing::Previous) | None => {}
        }
    }

    pub(crate) fn share_tiled_edge(&mut self, index: usize, rect: Rect) {
        let Some((tile, old)) = self.windows[index].tile else {
            return;
        };
        self.windows[index].tile = Some((tile, rect));
        let right = |rect: Rect| rect.location.x + rect.size.w;
        let bottom = |rect: Rect| rect.location.y + rect.size.h;
        for neighbor in 0..self.windows.len() {
            let Some((their_tile, theirs)) = self.windows[neighbor].tile else {
                continue;
            };
            if neighbor == index
                || !self.present(&self.windows[neighbor])
                || self.screen_of(&self.windows[neighbor]) != self.screen_of(&self.windows[index])
            {
                continue;
            }
            let mut shared = theirs;
            if tile.on_left() && !their_tile.on_left() && theirs.location.x == right(old) {
                shared.location.x = right(rect);
                shared.size.w = right(theirs) - right(rect);
            } else if !tile.on_left() && their_tile.on_left() && right(theirs) == old.location.x {
                shared.size.w = rect.location.x - theirs.location.x;
            } else if tile.on_left() == their_tile.on_left()
                && tile.row() == Row::Top
                && their_tile.row() == Row::Bottom
                && theirs.location.y == bottom(old)
            {
                shared.location.y = bottom(rect);
                shared.size.h = bottom(theirs) - bottom(rect);
            } else if tile.on_left() == their_tile.on_left()
                && tile.row() == Row::Bottom
                && their_tile.row() == Row::Top
                && bottom(theirs) == old.location.y
            {
                shared.size.h = rect.location.y - theirs.location.y;
            } else {
                continue;
            }
            let window = &mut self.windows[neighbor];
            window.tile = Some((their_tile, shared));
            window.request_size(Some(shared.size));
            if let Some(serial) = window.configure() {
                window.pending_location = Some((serial, shared.location));
            }
        }
    }

    pub(crate) fn track_snap(
        &mut self,
        surface: &WlSurface,
        pointer: Point<f64, Logical>,
        time: u32,
    ) {
        let now = Duration::from_millis(u64::from(time));
        let screen = self.pointer_screen();
        let candidate = snap_at(pointer, self.screen_rect(screen), self.usable_area(screen));
        let drag = self.snap.get_or_insert_with(|| SnapDrag {
            surface: surface.clone(),
            candidate: None,
            rest_at: pointer,
            since: now,
            shown: None,
        });
        let moved = (pointer.x - drag.rest_at.x).hypot(pointer.y - drag.rest_at.y) > REST_DISTANCE;
        if candidate != drag.candidate || moved {
            drag.candidate = candidate;
            drag.rest_at = pointer;
            drag.since = now;
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
        let due = drag.since + REST;
        if now < due {
            return Some(due);
        }
        drag.shown = Some(candidate);
        let (owner, at) = (drag.surface.clone(), drag.rest_at);
        let target = self.snap_rect(candidate);
        let target = Look::shown(Rectangle::new(
            target.location.to_f64(),
            target.size.to_f64(),
        ));
        let (from, spring) = if self.reduce_motion {
            (Look::hidden(target.rect), Spring::FADE)
        } else {
            (
                Look::hidden(Rectangle::new(at, (1.0, 1.0).into())),
                Spring::QUICK,
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
            Some(Snap::Tile(tile)) => {
                self.tile_window(index, tile);
                self.offer_other_half(index);
            }
            Some(Snap::Zoom) => self.maximize(surface),
            None => {}
        }
    }

    pub(crate) fn animate_preview(&mut self, time: Duration) {
        let Some(preview) = &mut self.preview else {
            return;
        };
        preview.motion.retarget(preview.target);
        preview.motion.advance(time, self.screens[0].refresh());

        if preview.target.alpha == 0.0 && preview.motion.is_settled() {
            self.preview = None;
        }
    }
}

impl Headless {
    pub fn snap_tick(&mut self, now: Duration) -> Option<Duration> {
        self.state.snap_due(now)
    }
}
