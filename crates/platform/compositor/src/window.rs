use smithay::desktop::utils::bbox_from_surface_tree;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Serial, Size};
use smithay::wayland::compositor::with_states;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::shell::xdg::{SurfaceCachedState, ToplevelSurface};

use smithay::backend::renderer::element::Id;

use bande_spring::Spring;

use crate::frame::Frame;
use crate::motion::{Look, Motion};
use crate::scene::{Casting, Drawn};
use crate::state::State;
use crate::tile::Tile;

pub struct Window {
    pub(crate) toplevel: ToplevelSurface,
    pub(crate) location: Point<i32, Logical>,
    pub(crate) size: Option<Size<i32, Logical>>,
    committed_size: Option<Size<i32, Logical>>,
    pub(crate) foreign: Option<ForeignToplevelHandle>,
    pub(crate) restore: Option<Rectangle<i32, Logical>>,
    pub(crate) pending_location: Option<(Serial, Point<i32, Logical>)>,
    pub(crate) acked: Option<Serial>,
    pub(crate) resize: Option<Resize>,
    pub(crate) minimized: Option<u64>,
    pub(crate) hidden: bool,
    pub(crate) shadow: Casting,
    pub(crate) veil: Id,
    pub(crate) motion: Option<Motion>,
    pub(crate) drawn: Option<Drawn>,
    pub(crate) tile: Option<(Tile, Rectangle<i32, Logical>)>,
    pub(crate) workspace: u64,
    pub(crate) recalled: Option<Point<i32, Logical>>,
    pub(crate) tag: String,
    pub(crate) membership: Option<crate::resume::Membership>,
    pub(crate) resumed: Option<crate::resume::Resumed>,
    pub(crate) resume_rank: Option<usize>,
    pub(crate) parent_anchor: Option<Point<i32, Logical>>,
    pub(crate) keep_on_top: bool,
    pub(crate) panel: bool,
    pub(crate) origin: Option<Rectangle<f64, Logical>>,
    pub(crate) frame: Frame,
    pub(crate) menu_button: Option<Rectangle<i32, Logical>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Resize {
    pub(crate) edges: xdg_toplevel::ResizeEdge,
    pub(crate) initial: Rectangle<i32, Logical>,
    pub(crate) active: bool,
}

const TOP: u32 = 1;
const BOTTOM: u32 = 2;
const LEFT: u32 = 4;
const RIGHT: u32 = 8;

const CONTROLS_OVER_CONTENT_APP: &str = "org.bande.Browser";

fn title_bar_height(mode: Option<DecorationMode>, fullscreen: bool) -> i32 {
    if mode == Some(DecorationMode::ServerSide) && !fullscreen {
        bande_graphics::FRAME_HEADER_HEIGHT as i32
    } else {
        0
    }
}

impl Resize {
    fn has(&self, edge: u32) -> bool {
        u32::from(self.edges) & edge != 0
    }

    pub(crate) fn size_for(&self, delta: Point<i32, Logical>) -> Size<i32, Logical> {
        let mut size = self.initial.size;
        if self.has(LEFT) {
            size.w -= delta.x;
        } else if self.has(RIGHT) {
            size.w += delta.x;
        }
        if self.has(TOP) {
            size.h -= delta.y;
        } else if self.has(BOTTOM) {
            size.h += delta.y;
        }
        size
    }

    pub(crate) fn location_for(&self, size: Size<i32, Logical>) -> Point<i32, Logical> {
        let mut location = self.initial.loc;
        if self.has(LEFT) {
            location.x += self.initial.size.w - size.w;
        }
        if self.has(TOP) {
            location.y += self.initial.size.h - size.h;
        }
        location
    }
}

impl Window {
    pub(crate) fn new(toplevel: ToplevelSurface, workspace: u64) -> Self {
        Self {
            toplevel,
            location: Point::default(),
            size: None,
            committed_size: None,
            foreign: None,
            restore: None,
            pending_location: None,
            acked: None,
            resize: None,
            minimized: None,
            hidden: false,
            shadow: Casting::new(),
            veil: Id::new(),
            motion: None,
            drawn: None,
            tile: None,
            workspace,
            recalled: None,
            membership: None,
            resumed: None,
            resume_rank: None,
            tag: String::new(),
            parent_anchor: None,
            keep_on_top: false,
            panel: false,
            origin: None,
            frame: Frame::default(),
            menu_button: None,
        }
    }

    pub fn location(&self) -> Point<i32, Logical> {
        self.location
    }

    pub fn size(&self) -> Option<Size<i32, Logical>> {
        self.size
    }

    pub fn geometry(&self) -> Option<Rectangle<i32, Logical>> {
        self.size?;
        let geometry = self.local_geometry();
        Some(Rectangle::new(
            self.location,
            (geometry.size.w, geometry.size.h + self.title_bar_height()).into(),
        ))
    }

    pub(crate) fn committed_frame(&self) -> Option<Rectangle<i32, Logical>> {
        self.committed_size
            .map(|size| Rectangle::new(self.location, size))
    }

    pub fn is_minimized(&self) -> bool {
        self.minimized.is_some()
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    pub fn tile(&self) -> Option<Rectangle<i32, Logical>> {
        self.tile.map(|(_, rect)| rect)
    }

    pub fn app_id(&self) -> String {
        crate::shell::toplevel_names(self.surface()).1
    }

    pub fn has_state(&self, state: xdg_toplevel::State) -> bool {
        self.toplevel.current_state().states.contains(state)
    }

    pub(crate) fn visible(&self) -> bool {
        self.size.is_some() && self.minimized.is_none() && !self.hidden
    }

    pub(crate) fn surface(&self) -> &WlSurface {
        self.toplevel.wl_surface()
    }

    pub(crate) fn shown_look(&self) -> Option<Look> {
        self.motion.as_ref().and_then(Motion::shown)
    }

    pub(crate) fn rendered(&self) -> bool {
        self.visible() || (self.size.is_some() && self.shown_look().is_some())
    }

    pub(crate) fn title_bar_height(&self) -> i32 {
        let current = self.toplevel.current_state();
        title_bar_height(
            current.decoration_mode,
            current.states.contains(xdg_toplevel::State::Fullscreen),
        )
    }

    pub(crate) fn controls_over_content_height(&self) -> i32 {
        let current = self.toplevel.current_state();
        let client_side = current.decoration_mode != Some(DecorationMode::ServerSide);
        let fullscreen = current.states.contains(xdg_toplevel::State::Fullscreen);
        if client_side
            && !fullscreen
            && crate::shell::toplevel_names(self.surface()).1 == CONTROLS_OVER_CONTENT_APP
        {
            bande_graphics::WINDOW_HEADER_HEIGHT as i32
        } else {
            0
        }
    }

    pub(crate) fn content_location(&self) -> Point<i32, Logical> {
        (self.location.x, self.location.y + self.title_bar_height()).into()
    }

    pub(crate) fn surface_origin(&self) -> Point<i32, Logical> {
        let offset = self.local_geometry().loc;
        self.content_location() - offset
    }

    pub(crate) fn request_size(&self, outer: Option<Size<i32, Logical>>) {
        self.toplevel.with_pending_state(|pending| {
            let bar = title_bar_height(
                pending.decoration_mode,
                pending.states.contains(xdg_toplevel::State::Fullscreen),
            );
            pending.size = outer.map(|size| (size.w, (size.h - bar).max(1)).into());
        });
    }

    pub(crate) fn requested_size(&self) -> Option<Size<i32, Logical>> {
        self.toplevel.with_pending_state(|pending| {
            let bar = title_bar_height(
                pending.decoration_mode,
                pending.states.contains(xdg_toplevel::State::Fullscreen),
            );
            pending.size.map(|size| (size.w, size.h + bar).into())
        })
    }

    pub(crate) fn local_geometry(&self) -> Rectangle<i32, Logical> {
        let surface = self.toplevel.wl_surface();
        let bounds = bbox_from_surface_tree(surface, (0, 0));
        with_states(surface, |states| {
            states
                .cached_state
                .get::<SurfaceCachedState>()
                .current()
                .geometry
        })
        .and_then(|geometry| geometry.intersection(bounds))
        .unwrap_or(bounds)
    }

    pub(crate) fn size_limits(&self) -> (Size<i32, Logical>, Size<i32, Logical>) {
        with_states(self.toplevel.wl_surface(), |states| {
            let mut cached = states.cached_state.get::<SurfaceCachedState>();
            let current = cached.current();
            (
                (current.min_size.w, current.min_size.h).into(),
                (current.max_size.w, current.max_size.h).into(),
            )
        })
    }

    pub(crate) fn fixed_size(&self) -> bool {
        let (min, max) = self.size_limits();
        min.w > 0 && min.h > 0 && min == max
    }

    pub(crate) fn chose_own_size(&self) -> bool {
        let floating = self.tile.is_none()
            && self.resize.is_none()
            && !self.has_state(xdg_toplevel::State::Maximized)
            && !self.has_state(xdg_toplevel::State::Fullscreen);
        floating
            && match self.toplevel.with_pending_state(|pending| pending.size) {
                None => true,
                Some(requested) => {
                    self.toplevel.current_state().size == Some(requested)
                        && self.local_geometry().size != requested
                }
            }
    }

    pub(crate) fn clamp_size(&self, outer: Size<i32, Logical>) -> Size<i32, Logical> {
        let (min, max) = self.size_limits();
        let bar = self.title_bar_height();
        let clamp = |value: i32, min: i32, max: i32| {
            let value = value.max(min).max(1);
            if max > 0 { value.min(max) } else { value }
        };
        (
            clamp(outer.w, min.w, max.w),
            clamp(outer.h - bar, min.h, max.h) + bar,
        )
            .into()
    }

    pub(crate) fn configure(&self) -> Option<Serial> {
        self.toplevel
            .is_initial_configure_sent()
            .then(|| self.toplevel.send_pending_configure())
            .flatten()
    }

    pub(crate) fn move_with_next_configure(&mut self, location: Point<i32, Logical>) {
        if let Some(serial) = self.configure() {
            self.pending_location = Some((serial, location));
        } else if let Some((_, at)) = &mut self.pending_location {
            *at = location;
        } else {
            self.location = location;
        }
    }
}

impl State {
    pub(crate) fn window_index(&self, surface: &WlSurface) -> Option<usize> {
        self.windows
            .iter()
            .position(|window| window.surface() == surface)
    }

    pub(crate) fn commit_window(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        if self.windows[index].size.is_none()
            && let Some(workspace) = self.parent_workspace(index)
        {
            self.windows[index].workspace = workspace;
        }
        let screen = self.screen_of(&self.windows[index]);
        let area = self.usable_area(screen);
        let output = self.screen_rect(screen);
        let siblings = if self.windows[index].size.is_none() {
            self.sibling_locations(index)
        } else {
            Vec::new()
        };
        if !self.windows[index].toplevel.is_initial_configure_sent() {
            self.windows[index].toplevel.with_pending_state(|state| {
                let bar = title_bar_height(state.decoration_mode, false);
                state.bounds = Some((area.size.w, area.size.h - bar).into());
            });
            if !self.resume_placement(index) {
                self.recall_frame(index);
            }
            self.windows[index].toplevel.send_configure();
            return;
        }
        let size = crate::state::mapped_size(surface);
        if size.is_none() && self.windows[index].size.is_some() {
            self.remember_frame(index);
        }
        let mapped = self.windows[index].size.is_some();
        self.windows[index].size = size;
        let before = self.windows[index].committed_frame();
        self.windows[index].committed_size =
            self.windows[index].geometry().map(|geometry| geometry.size);
        let over_parent = if mapped {
            None
        } else {
            self.dragged_window_at(surface)
                .or_else(|| self.place_over_parent(index, area))
        };
        let window = &mut self.windows[index];
        if let Some(geometry) = window.geometry() {
            let acked = window.acked;
            if !mapped {
                window.location = if window.has_state(xdg_toplevel::State::Fullscreen) {
                    output.loc
                } else if window.has_state(xdg_toplevel::State::Maximized) {
                    area.loc
                } else if let Some(at) = over_parent {
                    at
                } else if let Some(at) = window.recalled.take() {
                    crate::recall::clamp_into(Rectangle::new(at, geometry.size), area).loc
                } else if window.panel {
                    Point::from((
                        area.loc.x + (area.size.w - geometry.size.w).max(0) / 2,
                        area.loc.y + (area.size.h - geometry.size.h).max(0) / 2,
                    ))
                } else {
                    crate::cascade::cascaded(area, geometry.size, &siblings)
                };
                window.pending_location = None;
            } else if let Some((serial, at)) = window.pending_location
                && acked.is_some_and(|acked| acked >= serial)
            {
                window.location = at;
                window.pending_location = None;
            } else if let Some(resize) = window.resize {
                window.location = resize.location_for(geometry.size);
                if !resize.active && !window.has_state(xdg_toplevel::State::Resizing) {
                    window.resize = None;
                }
            }
        }
        if size.is_some() {
            if self.windows[index].chose_own_size() {
                self.follow_own_size(index, before);
            }
            if self.windows[index].fixed_size() {
                self.release_placement(index);
            }
        }
        if !mapped && size.is_some() {
            self.assign_new_window(index);
            self.open_window(index);
        }
        let surface = surface.clone();
        if size.is_some() {
            self.publish_toplevel(&surface);
        } else {
            self.withdraw_toplevel(&surface);
        }
        self.after_window_commit(&surface, mapped, size.is_some());
    }

    fn follow_own_size(&mut self, index: usize, before: Option<Rectangle<i32, Logical>>) {
        self.windows[index].request_size(None);
        let (Some(before), Some(after)) = (before, self.windows[index].geometry()) else {
            return;
        };
        if before.size == after.size {
            return;
        }
        let area = self.usable_area(self.screen_of(&self.windows[index]));
        if area.contains_rect(before) {
            self.windows[index].location = crate::recall::clamp_into(after, area).loc;
        }
        self.animate_window(index, Spring::STANDARD, Some(Look::shown(before.to_f64())));
    }

    fn release_placement(&mut self, index: usize) {
        let window = &self.windows[index];
        let surface = window.surface().clone();
        let (maximized, fullscreen) = window.toplevel.with_pending_state(|pending| {
            (
                pending.states.contains(xdg_toplevel::State::Maximized),
                pending.states.contains(xdg_toplevel::State::Fullscreen),
            )
        });
        if fullscreen {
            self.unfullscreen(&surface);
        } else if maximized {
            self.unmaximize(&surface);
        } else {
            self.untile(index);
        }
    }

    pub(crate) fn sync_activated(&mut self) {
        let key = self.key_window();
        for window in &self.windows {
            let active = key.as_ref() == Some(window.surface());
            window.toplevel.with_pending_state(|state| {
                if active {
                    state.states.set(xdg_toplevel::State::Activated);
                } else {
                    state.states.unset(xdg_toplevel::State::Activated);
                }
            });
            window.configure();
        }
        self.announce_active_app(key.as_ref());
    }

    pub(crate) fn suspend_unseen_windows(&self) {
        let locked = self.is_locked();
        for window in &self.windows {
            let unseen = locked
                || window.minimized.is_some()
                || window.hidden
                || self.on_hidden_workspace(window);
            let changed = window.toplevel.with_pending_state(|state| {
                if unseen {
                    state.states.set(xdg_toplevel::State::Suspended)
                } else {
                    state.states.unset(xdg_toplevel::State::Suspended)
                }
            });
            if changed {
                window.configure();
            }
        }
    }

    pub(crate) fn key_window(&self) -> Option<WlSurface> {
        let focused = match &self.exclusive_layer {
            Some(layer) => layer.previous.clone(),
            None => self.focused(),
        }?;
        self.toplevel_of(&focused).or_else(|| {
            self.key_behind_layer
                .clone()
                .filter(|key| self.window_index(key).is_some())
        })
    }

    pub(crate) fn place(
        &mut self,
        index: usize,
        state: xdg_toplevel::State,
        target: Option<Rectangle<i32, Logical>>,
    ) {
        let window = &mut self.windows[index];
        if window.restore.is_none()
            && window.tile.is_none()
            && let Some(geometry) = window.geometry()
            && !window.has_state(xdg_toplevel::State::Maximized)
            && !window.has_state(xdg_toplevel::State::Fullscreen)
        {
            window.restore = Some(geometry);
        }
        window.toplevel.with_pending_state(|pending| {
            for placed in [
                xdg_toplevel::State::Maximized,
                xdg_toplevel::State::Fullscreen,
            ] {
                if placed != state {
                    pending.states.unset(placed);
                }
            }
            match target {
                Some(_) => {
                    pending.states.set(state);
                }
                None => {
                    pending.states.unset(state);
                }
            }
        });
        window.request_size(target.or(window.restore).map(|rect| rect.size));
        let location = target.or(window.restore).map(|rect| rect.loc);
        if target.is_none() {
            window.restore = None;
        }
        if let Some(serial) = window.configure()
            && let Some(location) = location
        {
            window.pending_location = Some((serial, location));
        }
        if target.is_some() {
            self.set_tile(index, None);
        }
        self.animate_window(index, Spring::STANDARD, None);
    }

    pub(crate) fn maximize(&mut self, surface: &WlSurface) {
        if let Some(index) = self
            .window_index(surface)
            .filter(|index| !self.windows[*index].fixed_size())
        {
            let area = self.usable_area(self.screen_of(&self.windows[index]));
            self.place(index, xdg_toplevel::State::Maximized, Some(area));
        }
    }

    pub(crate) fn fit_to_usable_areas(&mut self) {
        for screen in 0..self.screens.len() {
            let area = self.usable_area(screen);
            let workspaces = &self.screens[screen].workspaces;
            let full_screen = workspaces
                .id_at(workspaces.active())
                .is_some_and(|id| workspaces.is_full_screen(id));
            if full_screen || self.screens[screen].fitted_to == area {
                continue;
            }
            self.screens[screen].fitted_to = area;
            for index in 0..self.windows.len() {
                let window = &self.windows[index];
                if self.screen_of(window) != screen
                    || self.on_full_screen_space(window)
                    || window.has_state(xdg_toplevel::State::Fullscreen)
                {
                    continue;
                }
                let (fitted, target) = match window.tile {
                    Some((tile, rect)) => (rect == tile.rect(area), Some(tile)),
                    None if window.has_state(xdg_toplevel::State::Maximized) => {
                        (window.geometry() == Some(area), None)
                    }
                    None => continue,
                };
                match (fitted, target) {
                    (true, _) => {}
                    (false, Some(tile)) => self.tile_window(index, tile, Spring::STANDARD),
                    (false, None) => {
                        let surface = window.surface().clone();
                        self.maximize(&surface);
                    }
                }
            }
        }
    }

    pub(crate) fn unmaximize(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let window = &self.windows[index];
        if window
            .toplevel
            .with_pending_state(|pending| pending.states.contains(xdg_toplevel::State::Fullscreen))
        {
            window.toplevel.with_pending_state(|pending| {
                pending.states.unset(xdg_toplevel::State::Maximized);
            });
            window.configure();
            return;
        }
        self.place(index, xdg_toplevel::State::Maximized, None);
    }

    pub(crate) fn fullscreen(&mut self, surface: &WlSurface) {
        if let Some(index) = self
            .window_index(surface)
            .filter(|index| !self.windows[*index].fixed_size())
        {
            let output = self.screen_rect(self.screen_of(&self.windows[index]));
            let was_maximized = self.windows[index].has_state(xdg_toplevel::State::Maximized);
            self.place(index, xdg_toplevel::State::Fullscreen, Some(output));
            self.windows[index].resize = None;
            if was_maximized {
                self.windows[index].toplevel.with_pending_state(|pending| {
                    pending.states.set(xdg_toplevel::State::Maximized);
                });
                self.windows[index].configure();
            }
            if self.windows[index].size.is_some() && !self.windows[index].panel {
                self.enter_full_screen_space(index);
            }
        }
    }

    pub(crate) fn unfullscreen(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let stays_maximized = self.windows[index]
            .toplevel
            .with_pending_state(|pending| pending.states.contains(xdg_toplevel::State::Maximized));
        if stays_maximized {
            let area = self.usable_area(self.screen_of(&self.windows[index]));
            self.place(index, xdg_toplevel::State::Maximized, Some(area));
        } else {
            self.place(index, xdg_toplevel::State::Fullscreen, None);
        }
        self.leave_full_screen_space(index);
        self.tidy_workspaces();
    }

    pub(crate) fn minimize(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let index = self.root_index(index);
        if self.windows[index].minimized.is_some() {
            return;
        }
        self.minimize_sequence += 1;
        self.windows[index].minimized = Some(self.minimize_sequence);
        self.announce_minimized(index, true);
        self.animate_window(index, Spring::GENTLE, None);
        let root = self.windows[index].surface().clone();
        self.keep_families_together();
        self.refocus_after_losing(&root);
    }

    pub(crate) fn unminimize(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let index = self.root_index(index);
        if self.windows[index].minimized.take().is_some() {
            self.announce_minimized(index, false);
            self.restore_window(index);
            self.keep_families_together();
        }
    }

    pub(crate) fn begin_resize(
        &mut self,
        surface: &WlSurface,
        edges: xdg_toplevel::ResizeEdge,
    ) -> bool {
        let Some(index) = self.window_index(surface) else {
            return false;
        };
        let window = &mut self.windows[index];
        if window.has_state(xdg_toplevel::State::Maximized)
            || window.has_state(xdg_toplevel::State::Fullscreen)
        {
            return false;
        }
        let Some(initial) = window.geometry() else {
            return false;
        };
        window.resize = Some(Resize {
            edges,
            initial,
            active: true,
        });
        window.toplevel.with_pending_state(|pending| {
            pending.states.set(xdg_toplevel::State::Resizing);
        });
        window.configure();
        true
    }

    pub(crate) fn resize_to(&mut self, surface: &WlSurface, delta: Point<i32, Logical>) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let window = &self.windows[index];
        let Some(resize) = window.resize else {
            return;
        };
        let size = window.clamp_size(resize.size_for(delta));
        let rect = Rectangle::new(resize.location_for(size), size);
        let rect = self.share_tiled_edge(index, rect);
        let window = &self.windows[index];
        window.request_size(Some(rect.size));
        window.configure();
    }

    pub(crate) fn end_resize(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let window = &mut self.windows[index];
        let started_at = window.resize.as_mut().map(|resize| {
            resize.active = false;
            resize.initial
        });
        window.toplevel.with_pending_state(|pending| {
            pending.states.unset(xdg_toplevel::State::Resizing);
        });
        window.configure();
        if let Some(started_at) = started_at {
            self.end_tile_if_reshaped(index, started_at);
        }
    }

    pub(crate) fn move_window_to(&mut self, surface: &WlSurface, location: Point<i32, Logical>) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let screen = self.pointer_screen();
        if screen != self.screen_of(&self.windows[index])
            && self.parent_index(index).is_none()
            && !self.on_full_screen_space(&self.windows[index])
        {
            let left = self.windows[index].workspace;
            self.windows[index].workspace = self.screens[screen].workspaces.active_id();
            self.window_left_workspace(left);
            self.sync_surface_outputs();
            self.announce_workspaces();
        }
        let top = self.usable_area(self.screen_of(&self.windows[index])).loc.y;
        let window = &mut self.windows[index];
        window.location = (location.x, location.y.max(top)).into();
        window.pending_location = None;
        self.needs_redraw = true;
        self.keep_families_together();
    }

    pub(crate) fn holds_place(&self, surface: &WlSurface) -> Option<bool> {
        let window = &self.windows[self.window_index(surface)?];
        if window.has_state(xdg_toplevel::State::Fullscreen) {
            return None;
        }
        Some(window.tile.is_some() || window.has_state(xdg_toplevel::State::Maximized))
    }

    pub(crate) fn begin_move(
        &mut self,
        surface: &WlSurface,
        pointer: Point<i32, Logical>,
    ) -> Option<Point<i32, Logical>> {
        let index = self.window_index(surface)?;
        let window = &self.windows[index];
        if window.has_state(xdg_toplevel::State::Fullscreen) {
            return None;
        }
        let geometry = window.geometry()?;
        let tiled = window.tile.is_some();
        if !tiled && !window.has_state(xdg_toplevel::State::Maximized) {
            return Some(geometry.loc);
        }
        let restored = window.restore.map_or(geometry.size, |restore| restore.size);
        let grabbed_at = pointer.x - geometry.loc.x;
        let x = pointer.x
            - i32::try_from(
                i64::from(grabbed_at) * i64::from(restored.w) / i64::from(geometry.size.w.max(1)),
            )
            .unwrap_or(0);
        let location = Point::from((x, geometry.loc.y));
        if tiled {
            self.untile(index);
        } else {
            self.place(index, xdg_toplevel::State::Maximized, None);
        }
        self.windows[index].motion = None;
        self.windows[index].pending_location = None;
        self.windows[index].location = location;
        Some(location)
    }
}
