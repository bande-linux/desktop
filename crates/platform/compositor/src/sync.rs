use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::time::Duration;

use smithay::backend::drm::DrmDeviceFd;
use smithay::delegate_drm_syncobj;
use smithay::reexports::calloop::{EventSource, Interest};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::{
    Blocker, BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker,
    add_pre_commit_hook, with_states,
};
use smithay::wayland::dmabuf::get_dmabuf;
use smithay::wayland::drm_syncobj::{
    DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd,
};

use crate::state::{Compositor, State};

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.globals.syncobj_state.as_mut()
    }
}

delegate_drm_syncobj!(State);

pub(crate) fn wait_for_fences(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _, surface| {
        let (acquire_point, dmabuf) = with_states(surface, |states| {
            let acquire_point = states
                .cached_state
                .get::<DrmSyncobjCachedState>()
                .pending()
                .acquire_point
                .clone();
            let dmabuf = match &states
                .cached_state
                .get::<SurfaceAttributes>()
                .pending()
                .buffer
            {
                Some(BufferAssignment::NewBuffer(buffer)) => get_dmabuf(buffer).ok().cloned(),
                _ => None,
            };
            (acquire_point, dmabuf)
        });
        if let Some(acquire_point) = acquire_point {
            if let Ok((blocker, source)) = acquire_point.generate_blocker() {
                state.block_until(surface, blocker, source);
            }
        } else if let Some(dmabuf) = dmabuf
            && let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ)
        {
            state.block_until(surface, blocker, source);
        }
    });
}

impl State {
    fn block_until<B, S>(&mut self, surface: &WlSurface, blocker: B, source: S)
    where
        B: Blocker + Send + 'static,
        S: EventSource<Event = (), Ret = Result<(), io::Error>> + 'static,
    {
        let Some(client) = surface.client() else {
            return;
        };
        let inserted = self.blockers.insert_source(source, move |_, _, state| {
            let display = state.display_handle.clone();
            state
                .client_compositor_state(&client)
                .blocker_cleared(state, &display);
            Ok(())
        });
        if inserted.is_ok() {
            add_blocker(surface, blocker);
        }
    }
}

impl Compositor {
    pub fn enable_explicit_sync(&mut self, device: DrmDeviceFd) -> bool {
        if !supports_syncobj_eventfd(&device) {
            return false;
        }
        let display = self.state.display_handle.clone();
        self.state.globals.syncobj_state = Some(DrmSyncobjState::new::<State>(&display, device));
        true
    }

    pub fn fence_fd(&self) -> io::Result<OwnedFd> {
        self.fences.as_fd().try_clone_to_owned()
    }

    pub(crate) fn clear_signalled_fences(&mut self) -> io::Result<()> {
        self.fences
            .dispatch(Some(Duration::ZERO), &mut self.state)
            .map_err(io::Error::other)
    }
}
