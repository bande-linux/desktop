use std::time::Duration;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Point, Rectangle};
use unbound_protocols::server::unbound_shell_v1::{self, UnboundShellV1};
use unbound_spring::{Animation, Spring, State as Phase};

use crate::gesture::Tracker;
use crate::motion::Look;
use crate::state::{Headless, State};
use crate::window::Window;

const SETTLED_VIEW: f64 = 1e-3;
const SETTLED_SPEED: f64 = 1e-2;
const SWIPE_PER_WORKSPACE: f64 = 350.0;
const MOMENTUM_SPEED: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Space {
    id: u64,
    home: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct Workspaces {
    spaces: Vec<Space>,
    active: usize,
    next_id: u64,
    view: f64,
    settle: Option<Animation>,
    last: Option<Duration>,
    tracker: Option<Tracker>,
    announced: (usize, usize),
}

impl Default for Workspaces {
    fn default() -> Self {
        Self {
            spaces: vec![Space { id: 0, home: None }],
            active: 0,
            next_id: 1,
            view: 0.0,
            settle: None,
            last: None,
            tracker: None,
            announced: (1, 0),
        }
    }
}

impl Workspaces {
    pub(crate) fn active_id(&self) -> u64 {
        self.spaces[self.active].id
    }

    pub(crate) fn count(&self) -> usize {
        self.spaces.len()
    }

    fn shown_view(&self) -> f64 {
        match (&self.settle, self.last) {
            (Some(settle), None) => settle.target(),
            _ => self.view,
        }
    }

    pub(crate) fn index_of(&self, id: u64) -> Option<usize> {
        self.spaces.iter().position(|space| space.id == id)
    }

    pub(crate) fn id_at(&self, index: usize) -> Option<u64> {
        self.spaces.get(index).map(|space| space.id)
    }

    pub(crate) fn is_full_screen(&self, id: u64) -> bool {
        self.spaces
            .iter()
            .any(|space| space.id == id && space.home.is_some())
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.settle.is_some()
    }

    fn is_settled(&self) -> bool {
        self.settle.is_none() && self.tracker.is_none()
    }

    fn insert(&mut self, index: usize, home: Option<u64>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.spaces.insert(index, Space { id, home });
        if index <= self.active {
            self.active += 1;
            self.shift_view(1.0);
        }
        id
    }

    fn shift_view(&mut self, by: f64) {
        self.view += by;
        if let Some(settle) = &mut self.settle {
            let state = settle.state();
            *settle = Animation::new(
                Spring::STANDARD,
                Phase {
                    position: state.position + by,
                    velocity: state.velocity,
                },
                settle.target() + by,
            );
        }
    }

    fn settle_to(&mut self, index: usize, velocity: f64, spring: Spring) {
        self.tracker = None;
        self.active = index;
        self.last = None;
        self.settle = Some(Animation::new(
            spring,
            Phase {
                position: self.view,
                velocity,
            },
            index as f64,
        ));
    }
}

impl State {
    pub(crate) fn present(&self, window: &Window) -> bool {
        window.visible() && window.workspace == self.workspaces.active_id()
    }

    pub(crate) fn workspace_offset(&self, window: &Window) -> f64 {
        self.workspaces
            .index_of(window.workspace)
            .map_or(f64::INFINITY, |index| {
                index as f64 - self.workspaces.shown_view()
            })
    }

    pub(crate) fn on_view(&self, window: &Window) -> bool {
        let output = self.output_rect();
        let output = Rectangle::new(output.location.to_f64(), output.size.to_f64());
        match self.presented(window) {
            Some(look) => look.alpha > 0.0 && look.rect.overlaps(output),
            None => true,
        }
    }

    pub(crate) fn on_full_screen_space(&self, window: &Window) -> bool {
        self.workspaces.is_full_screen(window.workspace)
    }

    pub(crate) fn place_on_workspace(&self, look: &mut Look, offset: f64) {
        if self.reduce_motion {
            look.alpha *= (1.0 - offset.abs()).max(0.0);
        } else {
            look.rect.loc.x += offset * f64::from(self.logical_output_size().w);
        }
    }

    pub(crate) fn presented(&self, window: &Window) -> Option<Look> {
        let geometry = window.geometry()?;
        let real = Rectangle::new(geometry.location.to_f64(), geometry.size.to_f64());
        let animated = window.shown_look();
        let offset = self.workspace_offset(window);
        let shift = self.desktop_shift(real);
        if animated.is_none() && offset == 0.0 && shift == Point::default() {
            return None;
        }
        let mut look = animated.unwrap_or(Look::shown(real));
        self.place_on_workspace(&mut look, offset);
        look.rect.loc += shift;
        Some(look)
    }

    fn workspace_spring(&self, velocity: f64) -> Spring {
        if self.reduce_motion {
            Spring::FADE
        } else if velocity.abs() > MOMENTUM_SPEED {
            Spring::MOMENTUM
        } else {
            Spring::STANDARD
        }
    }

    fn slide_to(&mut self, index: usize, velocity: f64) {
        let index = index.min(self.workspaces.count() - 1);
        self.hide_desktop();
        let spring = self.workspace_spring(velocity);
        self.workspaces.settle_to(index, velocity, spring);
        self.needs_redraw = true;
        self.announce_workspaces();
        self.refocus_pointer();
    }

    pub(crate) fn switch_workspace(&mut self, index: usize) {
        self.slide_to(index, 0.0);
        self.focus_front_of_workspace();
    }

    fn focus_front_of_workspace(&mut self) {
        if self.exclusive_layer.is_some() {
            return;
        }
        let front = self.frontmost(None);
        self.focus(front);
    }

    pub(crate) fn step_workspace(&mut self, forward: bool) {
        let active = self.workspaces.active;
        let target = if forward {
            active + 1
        } else {
            active.saturating_sub(1)
        };
        if target < self.workspaces.count() && target != active {
            self.switch_workspace(target);
        }
    }

    pub(crate) fn show_workspace_of(&mut self, surface: &WlSurface) {
        let Some(index) = self
            .window_index(surface)
            .and_then(|index| self.workspaces.index_of(self.windows[index].workspace))
        else {
            return;
        };
        if index != self.workspaces.active {
            self.slide_to(index, 0.0);
        }
    }

    fn carry(&mut self, index: usize, pages: f64) {
        if pages == 0.0 {
            return;
        }
        let shift = pages * f64::from(self.logical_output_size().w);
        match &mut self.windows[index].motion {
            Some(motion) => motion.translate(shift),
            None => {
                let from = self.windows[index].geometry().map(|rect| {
                    let mut look =
                        Look::shown(Rectangle::new(rect.location.to_f64(), rect.size.to_f64()));
                    look.rect.loc.x += shift;
                    look
                });
                self.animate_window(index, Spring::STANDARD, from);
            }
        }
    }

    fn move_window_to_space(&mut self, index: usize, target: u64) {
        let from = self.workspaces.index_of(self.windows[index].workspace);
        let to = self.workspaces.index_of(target);
        self.windows[index].workspace = target;
        if let (Some(from), Some(to)) = (from, to) {
            self.carry(index, from as f64 - to as f64);
        }
        self.needs_redraw = true;
    }

    fn space_beside(&mut self, forward: bool) -> Option<u64> {
        let active = self.workspaces.active;
        if !forward {
            return active
                .checked_sub(1)
                .and_then(|index| self.workspaces.id_at(index));
        }
        match self.workspaces.id_at(active + 1) {
            Some(id) => Some(id),
            None => Some(self.workspaces.insert(active + 1, None)),
        }
    }

    pub(crate) fn move_to_workspace(&mut self, surface: &WlSurface, forward: bool, follow: bool) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        if self.on_full_screen_space(&self.windows[index]) {
            return;
        }
        let Some(target) = self.space_beside(forward) else {
            return;
        };
        self.move_window_to_space(index, target);
        if follow {
            if let Some(target) = self.workspaces.index_of(target) {
                self.slide_to(target, 0.0);
            }
            self.focus(Some(surface.clone()));
        } else {
            self.refocus_on_workspace(surface);
        }
        self.tidy_workspaces();
    }

    fn refocus_on_workspace(&mut self, moved: &WlSurface) {
        if self.focused().as_ref() == Some(moved) {
            let front = self.frontmost(None);
            self.focus(front);
        }
    }

    pub(crate) fn assign_new_window(&mut self, index: usize) {
        let active = self.workspaces.active_id();
        let owner = self.windows[index].surface().client();
        let shares_full_screen = self.workspaces.is_full_screen(active)
            && self
                .windows
                .iter()
                .any(|window| window.workspace == active && window.surface().client() == owner);
        self.windows[index].workspace =
            if self.workspaces.is_full_screen(active) && !shares_full_screen {
                self.workspaces.spaces[self.workspaces.active]
                    .home
                    .filter(|home| self.workspaces.index_of(*home).is_some())
                    .unwrap_or_else(|| self.workspaces.spaces[0].id)
            } else {
                active
            };
        if self.windows[index].has_state(xdg_toplevel::State::Fullscreen) {
            self.enter_full_screen_space(index);
        }
        self.announce_workspaces();
    }

    pub(crate) fn enter_full_screen_space(&mut self, index: usize) {
        let home = self.windows[index].workspace;
        if self.workspaces.is_full_screen(home) {
            return;
        }
        let Some(at) = self.workspaces.index_of(home) else {
            return;
        };
        let space = self.workspaces.insert(at + 1, Some(home));
        self.move_window_to_space(index, space);
        if let Some(target) = self.workspaces.index_of(space) {
            self.slide_to(target, 0.0);
        }
    }

    pub(crate) fn leave_full_screen_space(&mut self, index: usize) {
        let space = self.windows[index].workspace;
        let Some(at) = self.workspaces.index_of(space) else {
            return;
        };
        let Some(home) = self.workspaces.spaces[at].home else {
            return;
        };
        let home = match self.workspaces.index_of(home) {
            Some(_) => home,
            None => self.workspaces.insert(at, None),
        };
        self.move_window_to_space(index, home);
        if self.workspaces.active_id() == space
            && let Some(target) = self.workspaces.index_of(home)
        {
            self.slide_to(target, 0.0);
        }
    }

    pub(crate) fn window_left_workspace(&mut self, space: u64) {
        let empty = !self
            .windows
            .iter()
            .any(|window| window.workspace == space && window.size.is_some());
        if empty
            && self.workspaces.active_id() == space
            && let Some(at) = self.workspaces.index_of(space)
            && let Some(home) = self.workspaces.spaces[at].home
        {
            let target = self
                .workspaces
                .index_of(home)
                .unwrap_or(at.saturating_sub(1));
            self.switch_workspace(target);
        }
        self.tidy_workspaces();
    }

    pub(crate) fn tidy_workspaces(&mut self) {
        if !self.workspaces.is_settled() {
            self.announce_workspaces();
            return;
        }
        let homes: Vec<u64> = self
            .workspaces
            .spaces
            .iter()
            .filter_map(|space| space.home)
            .collect();
        let mut index = 0;
        while index < self.workspaces.count() {
            let space = self.workspaces.spaces[index];
            let empty = !self
                .windows
                .iter()
                .any(|window| window.workspace == space.id && window.size.is_some());
            let regular = self
                .workspaces
                .spaces
                .iter()
                .filter(|space| space.home.is_none())
                .count();
            let removable = empty
                && index != self.workspaces.active
                && !homes.contains(&space.id)
                && (space.home.is_some() || regular > 1);
            if !removable {
                index += 1;
                continue;
            }
            self.workspaces.spaces.remove(index);
            if index < self.workspaces.active {
                self.workspaces.active -= 1;
                self.workspaces.shift_view(-1.0);
            }
            self.needs_redraw = true;
        }
        self.announce_workspaces();
    }

    pub(crate) fn animate_workspaces(&mut self, time: Duration) {
        let refresh = self.refresh;
        let workspaces = &mut self.workspaces;
        let Some(settle) = workspaces.settle.as_mut() else {
            return;
        };
        let step = workspaces
            .last
            .map_or(refresh, |last| time.saturating_sub(last));
        workspaces.last = Some(time);
        workspaces.view = settle.advance(step.as_secs_f64()).position;
        if settle.is_settled(SETTLED_VIEW, SETTLED_SPEED) {
            workspaces.view = workspaces.active as f64;
            workspaces.settle = None;
            workspaces.last = None;
            self.tidy_workspaces();
        }
        self.needs_redraw = true;
    }

    pub(crate) fn begin_workspace_swipe(&mut self, time: u32) {
        self.hide_desktop();
        let workspaces = &mut self.workspaces;
        workspaces.settle = None;
        workspaces.last = None;
        let last = (workspaces.count() - 1) as f64;
        workspaces.tracker = Some(Tracker::new(workspaces.view, time, last));
    }

    pub(crate) fn track_workspace_swipe(&mut self, dx: f64, time: u32) {
        let Some(tracker) = self.workspaces.tracker.as_mut() else {
            return;
        };
        let raw = tracker.raw() - dx / SWIPE_PER_WORKSPACE;
        tracker.track(raw, time);
        self.workspaces.view = tracker.position();
        self.needs_redraw = true;
    }

    pub(crate) fn end_workspace_swipe(&mut self, cancelled: bool) {
        let Some(tracker) = self.workspaces.tracker.take() else {
            return;
        };
        let (target, velocity) = if cancelled {
            (self.workspaces.active as f64, 0.0)
        } else {
            tracker.release()
        };
        let target = target.round().max(0.0) as usize;
        let changed = target != self.workspaces.active;
        self.slide_to(target, velocity);
        if changed {
            self.focus_front_of_workspace();
        }
    }

    pub(crate) fn announce_workspaces(&mut self) {
        let state = (self.workspaces.count(), self.workspaces.active);
        let changed = state != self.workspaces.announced;
        self.workspaces.announced = state;
        for index in 0..self.windows.len() {
            let position = self.workspaces.index_of(self.windows[index].workspace);
            if position == self.windows[index].announced_workspace {
                continue;
            }
            self.windows[index].announced_workspace = position;
            if let (Some(handle), Some(position)) = (&self.windows[index].foreign, position) {
                for shell in &self.shell_resources {
                    if shell.version() >= unbound_shell_v1::EVT_WINDOW_WORKSPACE_SINCE {
                        shell.window_workspace(handle.identifier(), position as u32);
                    }
                }
            }
        }
        if changed {
            for shell in &self.shell_resources {
                self.send_workspaces(shell);
            }
        }
    }

    fn send_workspaces(&self, shell: &UnboundShellV1) {
        if shell.version() >= unbound_shell_v1::EVT_WORKSPACES_SINCE {
            shell.workspaces(
                self.workspaces.count() as u32,
                self.workspaces.active as u32,
            );
        }
    }

    pub(crate) fn announce_workspaces_to(&self, shell: &UnboundShellV1) {
        self.send_workspaces(shell);
        if shell.version() < unbound_shell_v1::EVT_WINDOW_WORKSPACE_SINCE {
            return;
        }
        for window in &self.windows {
            if let (Some(handle), Some(position)) =
                (&window.foreign, self.workspaces.index_of(window.workspace))
            {
                shell.window_workspace(handle.identifier(), position as u32);
            }
        }
    }
}

impl Headless {
    pub fn workspace_count(&self) -> usize {
        self.state.workspaces.count()
    }

    pub fn active_workspace(&self) -> usize {
        self.state.workspaces.active
    }

    pub fn workspace_of(&self, window: &Window) -> Option<usize> {
        self.state.workspaces.index_of(window.workspace)
    }
}
