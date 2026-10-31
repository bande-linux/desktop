use bande_spring::Spring;
use smithay::delegate_xdg_dialog;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::xdg::dialog::XdgDialogHandler;
use smithay::wayland::shell::xdg::{ToplevelSurface, XdgToplevelSurfaceData};

use crate::motion::Look;
use crate::recall::clamp_into;
use crate::state::{State, centered};
use crate::window::Window;

const NUDGE: f64 = 16.0;
const FLASH_ALPHA: f64 = 0.5;

fn is_modal(window: &Window) -> bool {
    with_states(window.surface(), |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok().map(|role| role.modal))
            .unwrap_or(false)
    })
}

fn anchor(rect: Rectangle<i32, Logical>) -> Point<i32, Logical> {
    (rect.loc.x + rect.size.w / 2, rect.loc.y).into()
}

impl State {
    pub(crate) fn parent_index(&self, index: usize) -> Option<usize> {
        let parent = self.windows[index].toplevel.parent()?;
        self.window_index(&parent)
    }

    pub(crate) fn root_index(&self, mut index: usize) -> usize {
        while let Some(parent) = self.parent_index(index) {
            index = parent;
        }
        index
    }

    fn children(&self, index: usize) -> Vec<usize> {
        let parent = self.windows[index].surface();
        (0..self.windows.len())
            .filter(|&child| self.windows[child].toplevel.parent().as_ref() == Some(parent))
            .collect()
    }

    pub(crate) fn family_from(&self, index: usize) -> Vec<usize> {
        (0..self.windows.len())
            .filter(|&member| self.descends_from(member, index))
            .collect()
    }

    pub(crate) fn descends_from(&self, mut index: usize, ancestor: usize) -> bool {
        loop {
            if index == ancestor {
                return true;
            }
            match self.parent_index(index) {
                Some(parent) => index = parent,
                None => return false,
            }
        }
    }

    fn stack_family(&self, index: usize, front: usize, order: &mut Vec<WlSurface>) {
        order.push(self.windows[index].surface().clone());
        let (toward_front, rest): (Vec<usize>, Vec<usize>) = self
            .children(index)
            .into_iter()
            .partition(|&child| self.descends_from(front, child));
        for child in rest.into_iter().chain(toward_front) {
            self.stack_family(child, front, order);
        }
    }

    pub(crate) fn raise(&mut self, index: usize) {
        let mut order = Vec::new();
        self.stack_family(self.root_index(index), index, &mut order);
        for surface in order {
            if let Some(at) = self.window_index(&surface) {
                let window = self.windows.remove(at);
                self.windows.push(window);
            }
        }
        self.needs_redraw = true;
    }

    pub(crate) fn stack_above_parent(&mut self, index: usize) -> bool {
        let Some(parent) = self.parent_index(index) else {
            return false;
        };
        let window = self.windows.remove(index);
        let parent = if parent > index { parent - 1 } else { parent };
        let above = (parent..self.windows.len())
            .rev()
            .find(|&other| self.descends_from(other, parent))
            .unwrap_or(parent);
        self.windows.insert(above + 1, window);
        true
    }

    pub(crate) fn modal_blocker(&self, surface: &WlSurface) -> Option<WlSurface> {
        let start = self.window_index(surface)?;
        let mut index = start;
        while let Some(child) = self
            .children(index)
            .into_iter()
            .rev()
            .find(|&child| self.windows[child].visible() && is_modal(&self.windows[child]))
        {
            index = child;
        }
        (index != start).then(|| self.windows[index].surface().clone())
    }

    pub(crate) fn nudge(&mut self, surface: &WlSurface) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let Some(rect) = self.windows[index].geometry() else {
            return;
        };
        let mut from = Look::shown(rect.to_f64());
        if self.reduce_motion {
            from.alpha = FLASH_ALPHA;
        } else {
            from.rect.loc.x += NUDGE;
        }
        self.windows[index].motion = None;
        self.animate_window(index, Spring::SHAKE, Some(from));
    }

    pub(crate) fn place_over_parent(
        &self,
        index: usize,
        area: Rectangle<i32, Logical>,
    ) -> Option<Point<i32, Logical>> {
        let parent = self.windows[self.parent_index(index)?].geometry()?;
        let size = self.windows[index].geometry()?.size;
        let over = Rectangle::new(centered(parent, size), size);
        Some(clamp_into(over, area).loc)
    }

    pub(crate) fn parent_workspace(&self, index: usize) -> Option<u64> {
        self.parent_index(index)
            .map(|parent| self.windows[parent].workspace)
    }

    pub(crate) fn keep_families_together(&mut self) {
        let mut order: Vec<(usize, usize)> = (0..self.windows.len())
            .filter_map(|index| Some((index, self.parent_index(index)?)))
            .collect();
        order.sort_by_key(|&(index, _)| {
            std::iter::successors(Some(index), |&at| self.parent_index(at)).count()
        });
        let order: Vec<WlSurface> = order
            .into_iter()
            .map(|(child, _)| self.windows[child].surface().clone())
            .collect();
        let mut lost = Vec::new();
        let mut moved_workspace = false;
        for surface in order {
            let Some(child) = self.window_index(&surface) else {
                continue;
            };
            let Some(parent) = self.parent_index(child) else {
                continue;
            };
            if self.windows[parent].size.is_none() || self.windows[child].size.is_none() {
                continue;
            }
            let workspace = self.windows[parent].workspace;
            let left = self.windows[child].workspace;
            if left != workspace {
                self.windows[child].workspace = workspace;
                self.window_left_workspace(left);
                moved_workspace = true;
            }
            self.follow_minimized(child, parent, &mut lost);
            self.follow_hidden(child, parent, &mut lost);
            self.follow_location(child, parent);
        }
        if moved_workspace {
            self.sync_surface_outputs();
            self.announce_workspaces();
            self.needs_redraw = true;
        }
        for surface in lost {
            self.refocus_after_losing(&surface);
        }
        self.suspend_unseen_windows();
    }

    fn follow_minimized(&mut self, child: usize, parent: usize, lost: &mut Vec<WlSurface>) {
        let minimized = self.windows[parent].minimized;
        if minimized.is_some() == self.windows[child].minimized.is_some() {
            return;
        }
        self.windows[child].minimized = minimized;
        if minimized.is_some() {
            self.animate_window(child, Spring::GENTLE, None);
            lost.push(self.windows[child].surface().clone());
        } else {
            self.restore_window(child);
        }
    }

    fn follow_hidden(&mut self, child: usize, parent: usize, lost: &mut Vec<WlSurface>) {
        let hidden = self.windows[parent].hidden;
        if hidden == self.windows[child].hidden {
            return;
        }
        self.windows[child].hidden = hidden;
        if hidden {
            self.animate_window(child, Spring::FADE, None);
            lost.push(self.windows[child].surface().clone());
        } else {
            let from = self.windows[child]
                .geometry()
                .map(|rect| Look::hidden(rect.to_f64()));
            self.animate_window(child, Spring::FADE, from);
        }
    }

    fn follow_location(&mut self, child: usize, parent: usize) {
        let Some(rect) = self.windows[parent].geometry() else {
            return;
        };
        let now = anchor(rect);
        let parent_moving = self.windows[parent].motion.is_some();
        let window = &mut self.windows[child];
        let Some(before) = window.parent_anchor.replace(now) else {
            return;
        };
        let placed = window.tile.is_some()
            || window.has_state(xdg_toplevel::State::Maximized)
            || window.has_state(xdg_toplevel::State::Fullscreen);
        if before == now || placed {
            return;
        }
        let from = window.geometry().map(|rect| Look::shown(rect.to_f64()));
        let shift = now - before;
        window.location += shift;
        if let Some((_, location)) = &mut window.pending_location {
            *location += shift;
        }
        self.needs_redraw = true;
        if parent_moving {
            self.windows[child].motion = None;
            self.animate_window(child, Spring::STANDARD, from);
        }
    }
}

impl XdgDialogHandler for State {
    fn modal_changed(&mut self, toplevel: ToplevelSurface, is_modal: bool) {
        let Some(parent) = toplevel.parent() else {
            return;
        };
        if is_modal && self.key_window().as_ref() == Some(&parent) {
            self.focus(Some(toplevel.wl_surface().clone()));
        }
    }
}

delegate_xdg_dialog!(State);
