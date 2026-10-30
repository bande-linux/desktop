use smithay::input::Seat;
use smithay::input::SeatHandler;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent,
    GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
    GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab,
    PointerInnerHandle, RelativeMotionEvent,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Serial};

use crate::state::State;

#[derive(Debug, Clone, Copy)]
enum Operation {
    Move { origin: Point<i32, Logical> },
    Resize,
}

pub(crate) struct WindowGrab {
    start: GrabStartData<State>,
    surface: WlSurface,
    operation: Operation,
}

fn whole(point: Point<f64, Logical>) -> Point<i32, Logical> {
    (point.x.round() as i32, point.y.round() as i32).into()
}

impl WindowGrab {
    fn delta(&self, location: Point<f64, Logical>) -> Point<i32, Logical> {
        let delta = location - self.start.location;
        whole(delta)
    }
}

impl State {
    fn grab_start(
        &mut self,
        seat: &WlSeat,
        serial: Serial,
        surface: &WlSurface,
    ) -> Option<GrabStartData<State>> {
        let seat = Seat::<State>::from_resource(seat)?;
        let pointer = seat.get_pointer()?;
        if !pointer.has_grab(serial) {
            return None;
        }
        let start = pointer.grab_start_data()?;
        let (focus, _) = start.focus.as_ref()?;
        if focus.client() != surface.client() {
            return None;
        }
        Some(start)
    }

    pub(crate) fn request_move(&mut self, surface: &WlSurface, seat: &WlSeat, serial: Serial) {
        let Some(start) = self.grab_start(seat, serial, surface) else {
            return;
        };
        let Some(origin) = self.begin_move(surface, whole(start.location)) else {
            return;
        };
        self.start_grab(start, surface, Operation::Move { origin }, serial);
    }

    pub(crate) fn request_resize(
        &mut self,
        surface: &WlSurface,
        seat: &WlSeat,
        serial: Serial,
        edges: ResizeEdge,
    ) {
        let Some(start) = self.grab_start(seat, serial, surface) else {
            return;
        };
        if self.begin_resize(surface, edges) {
            self.start_grab(start, surface, Operation::Resize, serial);
        }
    }

    pub(crate) fn move_from_title_bar(&mut self, surface: &WlSurface, button: u32) {
        let location = self.pointer_location;
        let Some(origin) = self.begin_move(surface, whole(location)) else {
            return;
        };
        let start = GrabStartData {
            focus: None,
            button,
            location,
        };
        let serial = SERIAL_COUNTER.next_serial();
        self.start_grab(start, surface, Operation::Move { origin }, serial);
    }

    pub(crate) fn resize_from_frame(
        &mut self,
        surface: &WlSurface,
        edges: ResizeEdge,
        button: u32,
    ) {
        if !self.begin_resize(surface, edges) {
            return;
        }
        let start = GrabStartData {
            focus: None,
            button,
            location: self.pointer_location,
        };
        let serial = SERIAL_COUNTER.next_serial();
        self.start_grab(start, surface, Operation::Resize, serial);
    }

    fn start_grab(
        &mut self,
        start: GrabStartData<State>,
        surface: &WlSurface,
        operation: Operation,
        serial: Serial,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let grab = WindowGrab {
            start,
            surface: surface.clone(),
            operation,
        };
        pointer.set_grab(self, grab, serial, Focus::Clear);
    }
}

impl PointerGrab<State> for WindowGrab {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        let delta = self.delta(event.location);
        match self.operation {
            Operation::Move { origin } => {
                data.move_window_to(&self.surface, origin + delta);
                data.track_snap(&self.surface, event.location, event.time);
            }
            Operation::Resize => data.resize_to(&self.surface, delta),
        }
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        _focus: Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, None, event);
    }

    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        handle.gesture_swipe_begin(data, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        handle.gesture_swipe_update(data, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        handle.gesture_swipe_end(data, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        handle.gesture_pinch_begin(data, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        handle.gesture_pinch_update(data, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        handle.gesture_pinch_end(data, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        handle.gesture_hold_begin(data, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        handle.gesture_hold_end(data, event);
    }

    fn start_data(&self) -> &GrabStartData<State> {
        &self.start
    }

    fn unset(&mut self, data: &mut State) {
        match self.operation {
            Operation::Resize => data.end_resize(&self.surface),
            Operation::Move { .. } => data.finish_move(&self.surface),
        }
    }
}
