use std::sync::Mutex;

use smithay::backend::renderer::utils::{Buffer, RendererSurfaceStateUserData};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{
    BufferAssignment, SurfaceAttributes, SurfaceData, TraversalAction, add_destruction_hook,
    is_sync_subsurface, with_states, with_surface_tree_upward,
};

use crate::state::State;

#[derive(Default)]
struct ReplacedBuffer(Mutex<Option<Buffer>>);

pub(crate) fn release_on_destruction(surface: &WlSurface) {
    add_destruction_hook::<State, _>(surface, |_, surface| {
        drop(with_states(surface, take_replaced));
    });
}

pub(crate) fn hold_replaced_buffers(surface: &WlSurface) {
    if is_sync_subsurface(surface) {
        return;
    }
    with_surface_tree_upward(
        surface,
        (),
        |_, _, ()| TraversalAction::DoChildren(()),
        |_, states, ()| hold_replaced(states),
        |_, _, ()| true,
    );
}

fn hold_replaced(states: &SurfaceData) {
    let replaced = {
        let mut attributes = states.cached_state.get::<SurfaceAttributes>();
        let current = attributes.current();
        let Some(BufferAssignment::NewBuffer(new)) = &current.buffer else {
            return;
        };
        if current.frame_callbacks.is_empty() {
            return;
        }
        states
            .data_map
            .get::<RendererSurfaceStateUserData>()
            .and_then(|renderer| renderer.lock().ok()?.buffer().cloned())
            .filter(|old| **old != *new)
    };
    let Some(replaced) = replaced else {
        return;
    };
    states
        .data_map
        .insert_if_missing_threadsafe(ReplacedBuffer::default);
    if let Some(held) = states.data_map.get::<ReplacedBuffer>()
        && let Ok(mut held) = held.0.lock()
    {
        *held = Some(replaced);
    }
}

pub(crate) fn take_replaced(states: &SurfaceData) -> Option<Buffer> {
    states
        .data_map
        .get::<ReplacedBuffer>()
        .and_then(|held| held.0.lock().ok()?.take())
}
