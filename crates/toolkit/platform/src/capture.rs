use bande_graphics::{Captured, Event, Point, Rect};
use bande_widgets::PointerCapture;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1;
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibitor_v1::{
    self, ZwpKeyboardShortcutsInhibitorV1,
};
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

use crate::window::{State, Window, WindowError, WindowEvent};

#[derive(Default)]
pub(crate) struct Capture {
    pub(crate) inhibit_manager: Option<ZwpKeyboardShortcutsInhibitManagerV1>,
    pub(crate) constraints: Option<ZwpPointerConstraintsV1>,
    pub(crate) relative_manager: Option<ZwpRelativePointerManagerV1>,
    inhibitor: Option<ZwpKeyboardShortcutsInhibitorV1>,
    locked: Option<ZwpLockedPointerV1>,
    confined: Option<ZwpConfinedPointerV1>,
    relative: Option<ZwpRelativePointerV1>,
    active: bool,
}

impl Capture {
    fn end(&mut self) -> bool {
        if let Some(inhibitor) = self.inhibitor.take() {
            inhibitor.destroy();
        }
        if let Some(locked) = self.locked.take() {
            locked.destroy();
        }
        if let Some(confined) = self.confined.take() {
            confined.destroy();
        }
        if let Some(relative) = self.relative.take() {
            relative.destroy();
        }
        std::mem::take(&mut self.active)
    }

    pub(crate) fn release(&mut self) {
        self.end();
        if let Some(manager) = self.inhibit_manager.take() {
            manager.destroy();
        }
        if let Some(constraints) = self.constraints.take() {
            constraints.destroy();
        }
        if let Some(manager) = self.relative_manager.take() {
            manager.destroy();
        }
    }
}

impl State {
    fn stop_capture(&mut self) -> bool {
        let ended = self.capture.end();
        if ended {
            self.input.set_raw_keys(false);
            self.input.set_cursor_hidden(false);
        }
        ended
    }

    pub(crate) fn end_capture(&mut self) {
        if self.stop_capture() {
            self.events
                .push(WindowEvent::Input(Event::Captured(Captured::Ended)));
        }
    }
}

impl Window {
    pub fn capture_input(
        &mut self,
        area: Rect,
        pointer: PointerCapture,
    ) -> Result<(), WindowError> {
        self.state.stop_capture();
        let handle = self.queue.handle();
        let seat = self
            .state
            .seat
            .as_ref()
            .ok_or(WindowError::Missing("wl_seat"))?;
        let manager = self
            .state
            .capture
            .inhibit_manager
            .as_ref()
            .ok_or(WindowError::Missing(
                "zwp_keyboard_shortcuts_inhibit_manager_v1",
            ))?;
        let inhibitor = manager.inhibit_shortcuts(&self.surface, seat, &handle, ());
        self.state.capture.inhibitor = Some(inhibitor);
        self.state.capture.active = true;
        self.state.input.set_raw_keys(true);
        if pointer != PointerCapture::Free {
            self.constrain_pointer(area, pointer)?;
        }
        self.flush()
    }

    fn constrain_pointer(
        &mut self,
        area: Rect,
        pointer: PointerCapture,
    ) -> Result<(), WindowError> {
        let handle = self.queue.handle();
        let capture = &self.state.capture;
        let wl_pointer = self
            .state
            .input
            .wl_pointer()
            .ok_or(WindowError::Missing("wl_pointer"))?;
        let constraints = capture
            .constraints
            .clone()
            .ok_or(WindowError::Missing("zwp_pointer_constraints_v1"))?;
        let relative = match pointer {
            PointerCapture::Locked => Some(
                capture
                    .relative_manager
                    .as_ref()
                    .ok_or(WindowError::Missing("zwp_relative_pointer_manager_v1"))?
                    .get_relative_pointer(&wl_pointer, &handle, ()),
            ),
            PointerCapture::Confined | PointerCapture::Free => None,
        };
        let compositor = self
            .state
            .compositor
            .as_ref()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        let origin = self.content_origin();
        let region = compositor.create_region(&handle, ());
        region.add(
            (origin.x + area.x).round() as i32,
            (origin.y + area.y).round() as i32,
            area.width.round() as i32,
            area.height.round() as i32,
        );
        let capture = &mut self.state.capture;
        capture.relative = relative;
        match pointer {
            PointerCapture::Locked => {
                capture.locked = Some(constraints.lock_pointer(
                    &self.surface,
                    &wl_pointer,
                    Some(&region),
                    Lifetime::Oneshot,
                    &handle,
                    (),
                ));
            }
            PointerCapture::Confined => {
                capture.confined = Some(constraints.confine_pointer(
                    &self.surface,
                    &wl_pointer,
                    Some(&region),
                    Lifetime::Oneshot,
                    &handle,
                    (),
                ));
            }
            PointerCapture::Free => {}
        }
        region.destroy();
        Ok(())
    }

    pub fn release_input(&mut self) -> Result<(), WindowError> {
        if self.state.stop_capture() {
            self.flush()?;
        }
        Ok(())
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitorV1,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive = event {
            state.end_capture();
        }
    }
}

impl Dispatch<ZwpLockedPointerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpLockedPointerV1,
        event: zwp_locked_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_locked_pointer_v1::Event::Locked => state.input.set_cursor_hidden(true),
            zwp_locked_pointer_v1::Event::Unlocked => state.end_capture(),
            _ => {}
        }
    }
}

impl Dispatch<ZwpConfinedPointerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpConfinedPointerV1,
        event: zwp_confined_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_confined_pointer_v1::Event::Unconfined = event {
            state.end_capture();
        }
    }
}

impl Dispatch<ZwpRelativePointerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpRelativePointerV1,
        event: zwp_relative_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_relative_pointer_v1::Event::RelativeMotion {
            dx,
            dy,
            dx_unaccel,
            dy_unaccel,
            ..
        } = event
        {
            state
                .events
                .push(WindowEvent::Input(Event::Captured(Captured::Motion {
                    delta: Point::new(dx as f32, dy as f32),
                    unaccelerated: Point::new(dx_unaccel as f32, dy_unaccel as f32),
                })));
        }
    }
}

delegate_noop!(State: ignore ZwpKeyboardShortcutsInhibitManagerV1);
delegate_noop!(State: ignore ZwpPointerConstraintsV1);
delegate_noop!(State: ignore ZwpRelativePointerManagerV1);
