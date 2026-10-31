use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_confined_pointer_v1::{
    self, ZwpConfinedPointerV1,
};
use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_locked_pointer_v1::{
    self, ZwpLockedPointerV1,
};
use wayland_protocols::wp::pointer_constraints::zv1::client::zwp_pointer_constraints_v1::{
    Lifetime, ZwpPointerConstraintsV1,
};
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1;
use wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::{
    self, ZwpRelativePointerV1,
};

use crate::{Globals, Headless, TestClient};

#[derive(Default)]
pub(crate) struct Constraint {
    held: bool,
    relative: Vec<(f64, f64)>,
}

impl TestClient {
    pub fn use_relative_pointer(&mut self, server: &mut Headless) {
        let manager: ZwpRelativePointerManagerV1 = self.bind("zwp_relative_pointer_manager_v1", 1);
        let pointer = self.globals.input.pointer.clone().expect("a pointer");
        manager.get_relative_pointer(&pointer, &self.queue.handle(), ());
        self.exchange(server);
    }

    pub fn lock_pointer(&mut self, server: &mut Headless, window: usize) {
        let constraints: ZwpPointerConstraintsV1 = self.bind("zwp_pointer_constraints_v1", 1);
        let pointer = self.globals.input.pointer.clone().expect("a pointer");
        constraints.lock_pointer(
            &self.windows[window].0,
            &pointer,
            None,
            Lifetime::Persistent,
            &self.queue.handle(),
            (),
        );
        self.exchange(server);
    }

    pub fn confine_pointer(&mut self, server: &mut Headless, window: usize) {
        let constraints: ZwpPointerConstraintsV1 = self.bind("zwp_pointer_constraints_v1", 1);
        let pointer = self.globals.input.pointer.clone().expect("a pointer");
        constraints.confine_pointer(
            &self.windows[window].0,
            &pointer,
            None,
            Lifetime::Persistent,
            &self.queue.handle(),
            (),
        );
        self.exchange(server);
    }

    pub fn pointer_constrained(&self) -> bool {
        self.globals.constraint.held
    }

    pub fn relative_motions(&self) -> &[(f64, f64)] {
        &self.globals.constraint.relative
    }
}

impl Dispatch<ZwpLockedPointerV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpLockedPointerV1,
        event: zwp_locked_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_locked_pointer_v1::Event::Locked => globals.constraint.held = true,
            zwp_locked_pointer_v1::Event::Unlocked => globals.constraint.held = false,
            _ => {}
        }
    }
}

impl Dispatch<ZwpConfinedPointerV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpConfinedPointerV1,
        event: zwp_confined_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_confined_pointer_v1::Event::Confined => globals.constraint.held = true,
            zwp_confined_pointer_v1::Event::Unconfined => globals.constraint.held = false,
            _ => {}
        }
    }
}

impl Dispatch<ZwpRelativePointerV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpRelativePointerV1,
        event: zwp_relative_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion {
            dx_unaccel,
            dy_unaccel,
            ..
        } = event
        {
            globals.constraint.relative.push((dx_unaccel, dy_unaccel));
        }
    }
}

delegate_noop!(Globals: ZwpPointerConstraintsV1);
delegate_noop!(Globals: ZwpRelativePointerManagerV1);
