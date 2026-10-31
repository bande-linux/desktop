use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Point, Rectangle};
use unbound_protocols::server::unbound_shell_v1::UnboundShellV1;
use unbound_spring::{Animation, Spring, State as Phase};

use crate::gesture::Tracker;
use crate::motion::{Look, Tick, progress_precision};
use crate::state::{Headless, State};
use crate::window::Window;

const SWIPE_PER_WORKSPACE: f64 = 350.0;
const MOMENTUM_SPEED: f64 = 2.0;

static NEXT_SPACE: AtomicU64 = AtomicU64::new(0);

fn new_space_id() -> u64 {
    NEXT_SPACE.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Space {
    id: u64,
    home: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct Workspaces {
    spaces: Vec<Space>,
    active: usize,
    view: f64,
    settle: Option<Animation>,
    last: Option<Duration>,
    tracker: Option<Tracker>,
}

impl Default for Workspaces {
    fn default() -> Self {
        Self {
            spaces: vec![Space {
                id: new_space_id(),
                home: None,
            }],
            active: 0,
            view: 0.0,
            settle: None,
            last: None,
            tracker: None,
        }
    }
}

impl Workspaces {
    pub(crate) fn active_id(&self) -> u64 {
        self.spaces[self.active].id
    }

    pub(crate) fn active(&self) -> usize {
        self.active
    }

    pub(crate) fn count(&self) -> usize {
        self.spaces.len()
    }

    pub(crate) fn shown_view(&self) -> f64 {
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
        let id = new_space_id();
        self.spaces.insert(index, Space { id, home });
        if index <= self.active {
            self.active += 1;
            self.shift_view(1.0);
        }
        id
    }

    pub(crate) fn ids(&self) -> Vec<u64> {
        self.spaces.iter().map(|space| space.id).collect()
    }

    pub(crate) fn take(&mut self, ids: &[u64]) -> Vec<Space> {
        let active = self.active_id();
        let (taken, kept): (Vec<Space>, Vec<Space>) = self
            .spaces
            .drain(..)
            .partition(|space| ids.contains(&space.id));
        self.spaces = kept;
        if !self.spaces.iter().any(|space| space.home.is_none()) {
            self.spaces.insert(
                0,
                Space {
                    id: new_space_id(),
                    home: None,
                },
            );
        }
        let homes: Vec<u64> = self.spaces.iter().map(|space| space.id).collect();
        for space in &mut self.spaces {
            if space.home.is_some_and(|home| !homes.contains(&home)) {
                space.home = None;
            }
        }
        self.active = self
            .index_of(active)
            .unwrap_or(self.active.min(self.spaces.len() - 1));
        self.view = self.active as f64;
        self.settle = None;
        self.last = None;
        self.tracker = None;
        taken
    }

    pub(crate) fn adopt(&mut self, spaces: Vec<Space>) {
        self.spaces.extend(spaces);
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
        let velocity = match &self.settle {
            Some(settle) if velocity == 0.0 => settle.state().velocity,
            _ => velocity,
        };
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
        window.visible()
            && self
                .screens
                .iter()
                .any(|screen| screen.workspaces.active_id() == window.workspace)
    }

    pub(crate) fn workspace_offset(&self, window: &Window) -> f64 {
        let workspaces = &self.screens[self.screen_of(window)].workspaces;
        workspaces
            .index_of(window.workspace)
            .map_or(f64::INFINITY, |index| {
                index as f64 - workspaces.shown_view()
            })
    }

    pub(crate) fn on_view(&self, window: &Window) -> bool {
        let output = self.screen_rect(self.screen_of(window));
        let output = Rectangle::new(output.location.to_f64(), output.size.to_f64());
        match self.presented(window) {
            Some(look) => look.alpha > 0.0 && look.rect.overlaps(output),
            None => true,
        }
    }

    pub(crate) fn on_full_screen_space(&self, window: &Window) -> bool {
        self.screens
            .iter()
            .any(|screen| screen.workspaces.is_full_screen(window.workspace))
    }

    pub(crate) fn place_on_workspace(&self, look: &mut Look, offset: f64, screen: usize) {
        if self.reduce_motion {
            look.alpha *= (1.0 - offset.abs()).max(0.0);
        } else {
            look.rect.loc.x += offset * f64::from(self.screen_rect(screen).size.w);
        }
    }

    pub(crate) fn presented(&self, window: &Window) -> Option<Look> {
        let geometry = window.geometry()?;
        let real = Rectangle::new(geometry.location.to_f64(), geometry.size.to_f64());
        let animated = window.shown_look();
        let offset = self.workspace_offset(window);
        let screen = self.screen_of(window);
        let shift = self.desktop_shift(real, screen);
        let overview = self.overview.is_shown();
        if animated.is_none() && offset == 0.0 && shift == Point::default() && !overview {
            return None;
        }
        let mut look = animated.unwrap_or(Look::shown(real));
        self.place_on_workspace(&mut look, offset, screen);
        look.rect.loc += shift;
        if overview {
            look = self.in_overview(window, look);
        }
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

    fn slide_to(&mut self, screen: usize, index: usize, velocity: f64) {
        let index = index.min(self.screens[screen].workspaces.count() - 1);
        self.hide_desktop();
        self.carry_dragged_window(screen, index);
        let spring = self.workspace_spring(velocity);
        self.screens[screen]
            .workspaces
            .settle_to(index, velocity, spring);
        self.needs_redraw = true;
        self.announce_workspaces();
        self.refocus_pointer();
    }

    fn carry_dragged_window(&mut self, screen: usize, index: usize) {
        let Some(target) = self.screens[screen].workspaces.id_at(index) else {
            return;
        };
        let dragged = self
            .snap
            .as_ref()
            .and_then(|drag| self.window_index(drag.surface()));
        if let Some(dragged) = dragged
            && self.screen_of(&self.windows[dragged]) == screen
            && self.windows[dragged].workspace != target
            && !self.on_full_screen_space(&self.windows[dragged])
        {
            self.move_window_to_space(dragged, target);
        }
    }

    pub(crate) fn switch_workspace(&mut self, screen: usize, index: usize) {
        self.slide_to(screen, index, 0.0);
        self.focus_front_of_workspace(screen);
    }

    pub(crate) fn focus_front_of_workspace(&mut self, screen: usize) {
        if self.exclusive_layer.is_some() || self.overview.is_open() {
            return;
        }
        let front = self.frontmost_on(screen);
        self.focus(front);
    }

    pub(crate) fn step_workspace(&mut self, forward: bool) {
        let screen = self.pointer_screen();
        let workspaces = &self.screens[screen].workspaces;
        let active = workspaces.active;
        let target = if forward {
            active + 1
        } else {
            active.saturating_sub(1)
        };
        if target < workspaces.count() && target != active {
            self.switch_workspace(screen, target);
        }
    }

    pub(crate) fn show_workspace_of(&mut self, surface: &WlSurface) {
        let Some(window) = self.window_index(surface) else {
            return;
        };
        let screen = self.screen_of(&self.windows[window]);
        let workspaces = &self.screens[screen].workspaces;
        let Some(index) = workspaces.index_of(self.windows[window].workspace) else {
            return;
        };
        if index != workspaces.active {
            self.slide_to(screen, index, 0.0);
        }
    }

    fn carry(&mut self, index: usize, pages: f64) {
        if pages == 0.0 {
            return;
        }
        let width = self
            .screen_rect(self.screen_of(&self.windows[index]))
            .size
            .w;
        let shift = pages * f64::from(width);
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
        let workspaces = &self.screens[self.screen_of(&self.windows[index])].workspaces;
        let from = workspaces.index_of(self.windows[index].workspace);
        let to = workspaces.index_of(target);
        for member in self.family_from(index) {
            self.windows[member].workspace = target;
            if let (Some(from), Some(to)) = (from, to) {
                self.carry(member, from as f64 - to as f64);
            }
        }
        self.needs_redraw = true;
    }

    fn space_beside(&mut self, screen: usize, forward: bool) -> Option<u64> {
        let workspaces = &mut self.screens[screen].workspaces;
        let active = workspaces.active;
        if !forward {
            return active
                .checked_sub(1)
                .and_then(|index| workspaces.id_at(index));
        }
        match workspaces.id_at(active + 1) {
            Some(id) => Some(id),
            None => Some(workspaces.insert(active + 1, None)),
        }
    }

    pub(crate) fn move_to_workspace(&mut self, surface: &WlSurface, forward: bool, follow: bool) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let index = self.root_index(index);
        if self.on_full_screen_space(&self.windows[index]) {
            return;
        }
        let screen = self.screen_of(&self.windows[index]);
        let Some(target) = self.space_beside(screen, forward) else {
            return;
        };
        self.move_window_to_space(index, target);
        if follow {
            if let Some(target) = self.screens[screen].workspaces.index_of(target) {
                self.slide_to(screen, target, 0.0);
            }
            self.focus(Some(surface.clone()));
        } else {
            self.refocus_on_workspace(surface);
        }
        self.tidy_workspaces();
    }

    pub(crate) fn move_to_space_at(&mut self, surface: &WlSurface, screen: usize, position: usize) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let index = self.root_index(index);
        if self.on_full_screen_space(&self.windows[index]) {
            return;
        }
        let workspaces = &mut self.screens[screen].workspaces;
        let target = match workspaces.id_at(position) {
            Some(id) if !workspaces.is_full_screen(id) => id,
            Some(_) => return,
            None => workspaces.insert(workspaces.count(), None),
        };
        if self.screen_of(&self.windows[index]) == screen {
            self.move_window_to_space(index, target);
        } else {
            self.move_window_to_screen(index, screen, Some(target));
        }
        self.refocus_on_workspace(surface);
        self.announce_workspaces();
    }

    pub(crate) fn add_workspace(&mut self, screen: usize) {
        let workspaces = &mut self.screens[screen].workspaces;
        workspaces.insert(workspaces.count(), None);
        self.needs_redraw = true;
        self.announce_workspaces();
    }

    fn refocus_on_workspace(&mut self, moved: &WlSurface) {
        if self.focused().as_ref() == Some(moved) {
            let front = self.frontmost(None);
            self.focus(front);
        }
    }

    pub(crate) fn assign_new_window(&mut self, index: usize) {
        if let Some(workspace) = self.parent_workspace(index) {
            self.windows[index].workspace = workspace;
            self.announce_workspaces();
            return;
        }
        let screen = self.screen_of(&self.windows[index]);
        let workspaces = &self.screens[screen].workspaces;
        let active = workspaces.active_id();
        let owner = self.windows[index].surface().client();
        let shares_full_screen = workspaces.is_full_screen(active)
            && self.windows.iter().enumerate().any(|(other, window)| {
                other != index
                    && window.size.is_some()
                    && window.workspace == active
                    && window.surface().client() == owner
            });
        self.windows[index].workspace = if workspaces.is_full_screen(active) && !shares_full_screen
        {
            workspaces.spaces[workspaces.active]
                .home
                .filter(|home| workspaces.index_of(*home).is_some())
                .unwrap_or_else(|| workspaces.spaces[0].id)
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
        let screen = self.screen_of(&self.windows[index]);
        let workspaces = &mut self.screens[screen].workspaces;
        if workspaces.is_full_screen(home) {
            return;
        }
        let Some(at) = workspaces.index_of(home) else {
            return;
        };
        let space = workspaces.insert(at + 1, Some(home));
        self.move_window_to_space(index, space);
        if let Some(target) = self.screens[screen].workspaces.index_of(space) {
            self.slide_to(screen, target, 0.0);
        }
    }

    pub(crate) fn leave_full_screen_space(&mut self, index: usize) {
        let space = self.windows[index].workspace;
        let screen = self.screen_of(&self.windows[index]);
        let workspaces = &mut self.screens[screen].workspaces;
        let Some(at) = workspaces.index_of(space) else {
            return;
        };
        let Some(home) = workspaces.spaces[at].home else {
            return;
        };
        let home = match workspaces.index_of(home) {
            Some(_) => home,
            None => workspaces.insert(at, None),
        };
        self.move_window_to_space(index, home);
        let workspaces = &self.screens[screen].workspaces;
        if workspaces.active_id() == space
            && let Some(target) = workspaces.index_of(home)
        {
            self.slide_to(screen, target, 0.0);
        }
    }

    pub(crate) fn window_left_workspace(&mut self, space: u64) {
        let screen = self.screen_of_workspace(space);
        let empty = !self
            .windows
            .iter()
            .any(|window| window.workspace == space && window.size.is_some());
        let workspaces = &self.screens[screen].workspaces;
        if empty
            && workspaces.active_id() == space
            && let Some(at) = workspaces.index_of(space)
            && let Some(home) = workspaces.spaces[at].home
        {
            let target = workspaces.index_of(home).unwrap_or(at.saturating_sub(1));
            self.switch_workspace(screen, target);
        }
        self.tidy_workspaces();
    }

    pub(crate) fn tidy_workspaces(&mut self) {
        if !self.overview.is_open() {
            for screen in 0..self.screens.len() {
                self.tidy_workspaces_of(screen);
            }
        }
        self.announce_workspaces();
    }

    fn tidy_workspaces_of(&mut self, screen: usize) {
        let occupied: Vec<u64> = self
            .windows
            .iter()
            .filter(|window| window.size.is_some())
            .map(|window| window.workspace)
            .collect();
        let workspaces = &mut self.screens[screen].workspaces;
        if !workspaces.is_settled() {
            return;
        }
        let homes: Vec<u64> = workspaces
            .spaces
            .iter()
            .filter_map(|space| space.home)
            .collect();
        let mut index = 0;
        while index < workspaces.count() {
            let space = workspaces.spaces[index];
            let regular = workspaces
                .spaces
                .iter()
                .filter(|space| space.home.is_none())
                .count();
            let removable = !occupied.contains(&space.id)
                && index != workspaces.active
                && !homes.contains(&space.id)
                && (space.home.is_some() || regular > 1);
            if !removable {
                index += 1;
                continue;
            }
            workspaces.spaces.remove(index);
            if index < workspaces.active {
                workspaces.active -= 1;
                workspaces.shift_view(-1.0);
            }
            self.needs_redraw = true;
        }
    }

    pub(crate) fn animate_workspaces(&mut self, tick: Tick) {
        let mut settled = false;
        for screen in &mut self.screens {
            let tick = Tick {
                refresh: screen.refresh(),
                ..tick
            };
            let precision = progress_precision(screen.mode_size());
            let workspaces = &mut screen.workspaces;
            let Some(settle) = workspaces.settle.as_mut() else {
                continue;
            };
            let step = tick.step_since(workspaces.last);
            workspaces.last = Some(tick.time);
            workspaces.view = settle.advance(step).position;
            if settle.is_at_rest(precision) {
                workspaces.view = workspaces.active as f64;
                workspaces.settle = None;
                workspaces.last = None;
                settled = true;
            }
            self.needs_redraw = true;
        }
        if settled {
            self.tidy_workspaces();
        }
    }

    pub(crate) fn workspaces_animating(&self) -> bool {
        self.screens
            .iter()
            .any(|screen| screen.workspaces.is_animating())
    }

    pub(crate) fn begin_workspace_swipe(&mut self, time: u32) {
        self.hide_desktop();
        let screen = self.pointer_screen();
        let workspaces = &mut self.screens[screen].workspaces;
        workspaces.settle = None;
        workspaces.last = None;
        let last = (workspaces.count() - 1) as f64;
        workspaces.tracker = Some(Tracker::new(workspaces.view, time, last));
    }

    fn swiping_screen(&self) -> Option<usize> {
        self.screens
            .iter()
            .position(|screen| screen.workspaces.tracker.is_some())
    }

    pub(crate) fn track_workspace_swipe(&mut self, dx: f64, time: u32) {
        let Some(screen) = self.swiping_screen() else {
            return;
        };
        let workspaces = &mut self.screens[screen].workspaces;
        let Some(tracker) = workspaces.tracker.as_mut() else {
            return;
        };
        let raw = tracker.raw() - dx / SWIPE_PER_WORKSPACE;
        tracker.track(raw, time);
        workspaces.view = tracker.position();
        self.needs_redraw = true;
    }

    pub(crate) fn end_workspace_swipe(&mut self, cancelled: bool) {
        let Some(screen) = self.swiping_screen() else {
            return;
        };
        let workspaces = &mut self.screens[screen].workspaces;
        let Some(tracker) = workspaces.tracker.take() else {
            return;
        };
        let (target, velocity) = if cancelled {
            (workspaces.active as f64, 0.0)
        } else {
            tracker.release()
        };
        let target = target.round().max(0.0) as usize;
        let changed = target != workspaces.active;
        self.slide_to(screen, target, velocity);
        if changed {
            self.focus_front_of_workspace(screen);
        }
    }

    fn announced_screen(&self) -> usize {
        self.key_window()
            .and_then(|key| self.window_index(&key))
            .map_or_else(
                || self.pointer_screen(),
                |index| self.screen_of(&self.windows[index]),
            )
    }

    fn window_position(&self, window: &Window) -> Option<usize> {
        self.screens[self.screen_of(window)]
            .workspaces
            .index_of(window.workspace)
    }

    pub(crate) fn announce_workspaces(&mut self) {
        let workspaces = &self.screens[self.announced_screen()].workspaces;
        let state = (workspaces.count(), workspaces.active);
        let changed = state != self.announced_workspaces;
        self.announced_workspaces = state;
        if changed {
            for shell in &self.shell_resources {
                self.send_workspaces(shell);
            }
        }
        self.announce_full_screen();
        let key = self.key_window();
        self.announce_active_app(key.as_ref());
    }

    pub(crate) fn send_workspaces(&self, shell: &UnboundShellV1) {
        let (count, active) = self.announced_workspaces;
        shell.workspaces(count as u32, active as u32);
    }
}

impl Headless {
    pub fn workspace_count(&self) -> usize {
        self.state.main_screen().workspaces.count()
    }

    pub fn active_workspace(&self) -> usize {
        self.state.main_screen().workspaces.active
    }

    pub fn workspace_count_on(&self, output: &str) -> Option<usize> {
        let screen = self.state.screen_named(output)?;
        Some(self.state.screens[screen].workspaces.count())
    }

    pub fn active_workspace_on(&self, output: &str) -> Option<usize> {
        let screen = self.state.screen_named(output)?;
        Some(self.state.screens[screen].workspaces.active)
    }

    pub fn workspace_of(&self, window: &Window) -> Option<usize> {
        self.state.window_position(window)
    }
}
