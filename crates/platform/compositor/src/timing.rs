use std::time::Duration;

use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::{Monotonic, Time};
use smithay::wayland::commit_timing::{
    CommitTimerBarrierStateUserData, CommitTimerStateUserData, Timestamp,
};
use smithay::wayland::compositor::{
    Barrier, CompositorHandler, SurfaceData, TraversalAction, add_pre_commit_hook, with_states,
    with_surface_tree_downward,
};
use smithay::wayland::fifo::{FifoBarrierCachedState, FifoCachedState};
use smithay::{delegate_commit_timing, delegate_fifo};

use crate::state::{Compositor, State};

delegate_fifo!(State);
delegate_commit_timing!(State);

pub(crate) struct LatchedBarrier {
    barrier: Barrier,
    client: Option<Client>,
}

pub(crate) fn wait_for_timed_commits(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _, surface| {
        let waits = with_states(surface, |states| {
            states
                .cached_state
                .get::<FifoCachedState>()
                .pending()
                .wait_barrier
                || states
                    .data_map
                    .get::<CommitTimerStateUserData>()
                    .is_some_and(|timer| timer.borrow().timestamp.is_some())
        });
        if waits {
            state.needs_redraw = true;
        }
    });
}

pub(crate) fn take_barrier(surface: &WlSurface, states: &SurfaceData) -> Option<LatchedBarrier> {
    let barrier = states
        .cached_state
        .get::<FifoBarrierCachedState>()
        .current()
        .barrier
        .take()?;
    Some(LatchedBarrier {
        barrier,
        client: surface.client(),
    })
}

fn each_surface(roots: &[WlSurface], mut visit: impl FnMut(&WlSurface, &SurfaceData)) {
    for root in roots {
        with_surface_tree_downward(
            root,
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |surface, states, _| visit(surface, states),
            |_, _, _| true,
        );
    }
}

impl State {
    pub(crate) fn signal_barriers(&mut self, barriers: Vec<LatchedBarrier>) {
        let mut clients: Vec<Client> = Vec::new();
        for latched in barriers {
            latched.barrier.signal();
            if let Some(client) = latched.client
                && !clients.contains(&client)
            {
                clients.push(client);
            }
        }
        self.clear_blockers_of(clients);
    }

    fn clear_blockers_of(&mut self, clients: Vec<Client>) {
        let display = self.display_handle.clone();
        for client in clients {
            self.client_compositor_state(&client)
                .blocker_cleared(self, &display);
        }
    }

    pub(crate) fn hidden_barriers(
        &self,
        screen: usize,
        shown: &[WlSurface],
    ) -> Vec<LatchedBarrier> {
        let hidden: Vec<WlSurface> = self
            .windows
            .iter()
            .filter(|window| self.screen_of(window) == screen)
            .map(|window| window.surface().clone())
            .filter(|surface| !shown.contains(surface))
            .collect();
        let mut barriers = Vec::new();
        each_surface(&hidden, |surface, states| {
            barriers.extend(take_barrier(surface, states));
        });
        barriers
    }

    pub(crate) fn release_timed_commits(&mut self, screen: usize, shown_at: Duration) {
        let target = Timestamp::from(Time::<Monotonic>::from(shown_at));
        let mut roots = self.latch_roots(screen);
        roots.extend(
            self.windows
                .iter()
                .filter(|window| self.screen_of(window) == screen)
                .map(|window| window.surface().clone()),
        );
        let mut clients: Vec<Client> = Vec::new();
        let mut waiting = false;
        each_surface(&roots, |surface, states| {
            let Some(timer) = states.data_map.get::<CommitTimerBarrierStateUserData>() else {
                return;
            };
            let Ok(mut timer) = timer.lock() else {
                return;
            };
            if timer.signal_until(target)
                && let Some(client) = surface.client()
                && !clients.contains(&client)
            {
                clients.push(client);
            }
            waiting |= timer.next_deadline().is_some();
        });
        self.clear_blockers_of(clients);
        if waiting {
            self.screens[screen].redraw = true;
        }
    }
}

impl Compositor {
    pub fn release_timed_commits_on(&mut self, output: &str, shown_at: Duration) {
        if let Some(screen) = self.state.screen_named(output) {
            self.state.release_timed_commits(screen, shown_at);
        }
    }
}
