use smithay::desktop::utils::bbox_from_surface_tree;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Serial, Size};
use smithay::wayland::compositor::with_states;
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::shell::xdg::{SurfaceCachedState, ToplevelSurface};

use smithay::backend::renderer::element::Id;

use unbound_spring::Spring;

use crate::frame::Frame;
use crate::motion::{Look, Motion};
use crate::scene::{Casting, Drawn};
use crate::state::{Rect, State};
use crate::tile::{TILED, Tile};

pub struct Window {
    pub(crate) toplevel: ToplevelSurface,
    pub(crate) location: Point<i32, Logical>,
    pub(crate) size: Option<Size<i32, Logical>>,
    pub(crate) foreign: Option<ForeignToplevelHandle>,
    pub(crate) restore: Option<Rect>,
    pub(crate) pending_location: Option<(Serial, Point<i32, Logical>)>,
    pub(crate) acked: Option<Serial>,
    pub(crate) resize: Option<Resize>,
    pub(crate) minimized: Option<u64>,
    pub(crate) hidden: bool,
    pub(crate) shadow: Casting,
    pub(crate) veil: Id,
    pub(crate) motion: Option<Motion>,
    pub(crate) drawn: Option<Drawn>,
    pub(crate) tile: Option<(Tile, Rect)>,
    pub(crate) workspace: u64,
    pub(crate) recalled: Option<Point<i32, Logical>>,
    pub(crate) keep_on_top: bool,
    pub(crate) announced_workspace: Option<usize>,
    pub(crate) frame: Frame,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Resize {
    pub(crate) edges: xdg_toplevel::ResizeEdge,
    pub(crate) initial: Rect,
    pub(crate) active: bool,
}

const TOP: u32 = 1;
const BOTTOM: u32 = 2;
const LEFT: u32 = 4;
const RIGHT: u32 = 8;

fn title_bar_height(mode: Option<DecorationMode>, fullscreen: bool) -> i32 {
    if mode == Some(DecorationMode::ServerSide) && !fullscreen {
        unbound_graphics::FRAME_HEADER_HEIGHT as i32
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
        let mut location = self.initial.location;
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
            keep_on_top: false,
            announced_workspace: None,
            frame: Frame::default(),
        }
    }

    pub fn location(&self) -> Point<i32, Logical> {
        self.location
    }

    pub fn size(&self) -> Option<Size<i32, Logical>> {
        self.size
    }

    pub fn geometry(&self) -> Option<Rect> {
        self.size?;
        let geometry = self.local_geometry();
        Some(Rect {
            location: self.location,
            size: (geometry.size.w, geometry.size.h + self.title_bar_height()).into(),
        })
    }

    pub fn is_minimized(&self) -> bool {
        self.minimized.is_some()
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden
    }

    pub fn tile(&self) -> Option<Rect> {
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
        with_states(surface, |states| {
            states
                .cached_state
                .get::<SurfaceCachedState>()
                .current()
                .geometry
        })
        .unwrap_or_else(|| bbox_from_surface_tree(surface, (0, 0)))
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
            self.recall_frame(index);
            self.windows[index].toplevel.send_configure();
            return;
        }
        let size = crate::state::mapped_size(surface);
        if size.is_none() && self.windows[index].size.is_some() {
            self.remember_frame(index);
        }
        let window = &mut self.windows[index];
        let mapped = window.size.is_some();
        window.size = size;
        if let Some(geometry) = window.geometry() {
            let acked = window.acked;
            if !mapped {
                window.location = if window.has_state(xdg_toplevel::State::Fullscreen) {
                    output.location
                } else if window.has_state(xdg_toplevel::State::Maximized) {
                    area.location
                } else if let Some(at) = window.recalled.take() {
                    crate::recall::clamp_into(
                        Rect {
                            location: at,
                            size: geometry.size,
                        },
                        area,
                    )
                    .location
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

    fn place(&mut self, index: usize, state: xdg_toplevel::State, target: Option<Rect>) {
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
                    for tiled in TILED {
                        pending.states.unset(tiled);
                    }
                    pending.states.set(state);
                }
                None => {
                    pending.states.unset(state);
                }
            }
        });
        window.request_size(target.or(window.restore).map(|rect| rect.size));
        if target.is_some() {
            window.tile = None;
        }
        let location = target.or(window.restore).map(|rect| rect.location);
        if target.is_none() {
            window.restore = None;
        }
        if let Some(serial) = window.configure()
            && let Some(location) = location
        {
            window.pending_location = Some((serial, location));
        }
        self.animate_window(index, Spring::STANDARD, None);
    }

    pub(crate) fn maximize(&mut self, surface: &WlSurface) {
        if let Some(index) = self.window_index(surface) {
            let area = self.usable_area(self.screen_of(&self.windows[index]));
            self.place(index, xdg_toplevel::State::Maximized, Some(area));
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
        if let Some(index) = self.window_index(surface) {
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
            if self.windows[index].size.is_some() {
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

    pub(crate) fn toggle_fullscreen(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let fullscreen = self.windows[index]
            .toplevel
            .with_pending_state(|pending| pending.states.contains(xdg_toplevel::State::Fullscreen));
        if fullscreen {
            self.unfullscreen(surface);
        } else {
            self.fullscreen(surface);
        }
    }

    pub(crate) fn minimize(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        if self.windows[index].minimized.is_some() {
            return;
        }
        self.minimize_sequence += 1;
        self.windows[index].minimized = Some(self.minimize_sequence);
        self.announce_minimized(index, true);
        self.animate_window(index, Spring::GENTLE, None);
        self.refocus_after_losing(surface);
    }

    pub(crate) fn unminimize(&mut self, surface: &WlSurface) {
        if let Some(index) = self.window_index(surface)
            && self.windows[index].minimized.take().is_some()
        {
            self.announce_minimized(index, false);
            self.restore_window(index);
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
        window.request_size(Some(size));
        window.configure();
        let rect = Rect {
            location: resize.location_for(size),
            size,
        };
        self.share_tiled_edge(index, rect);
    }

    pub(crate) fn end_resize(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let window = &mut self.windows[index];
        if let Some(resize) = &mut window.resize {
            resize.active = false;
        }
        window.toplevel.with_pending_state(|pending| {
            pending.states.unset(xdg_toplevel::State::Resizing);
        });
        window.configure();
    }

    pub(crate) fn move_window_to(&mut self, surface: &WlSurface, location: Point<i32, Logical>) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let screen = self.pointer_screen();
        if screen != self.screen_of(&self.windows[index])
            && !self.on_full_screen_space(&self.windows[index])
        {
            let left = self.windows[index].workspace;
            self.windows[index].workspace = self.screens[screen].workspaces.active_id();
            self.window_left_workspace(left);
            self.sync_surface_outputs();
            self.announce_workspaces();
        }
        let top = self
            .usable_area(self.screen_of(&self.windows[index]))
            .location
            .y;
        let window = &mut self.windows[index];
        window.location = (location.x, location.y.max(top)).into();
        window.pending_location = None;
        self.needs_redraw = true;
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
            return Some(geometry.location);
        }
        let restored = window.restore.map_or(geometry.size, |restore| restore.size);
        let grabbed_at = pointer.x - geometry.location.x;
        let x = pointer.x
            - i32::try_from(
                i64::from(grabbed_at) * i64::from(restored.w) / i64::from(geometry.size.w.max(1)),
            )
            .unwrap_or(0);
        let location = Point::from((x, geometry.location.y));
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
