use smithay::input::Seat;
use smithay::input::SeatHandler;
use smithay::input::pointer::{
    AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent,
    GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent,
    GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData, MotionEvent, PointerGrab,
    PointerInnerHandle, RelativeMotionEvent,
};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, Logical, Monotonic, Point, Rectangle, SERIAL_COUNTER, Serial};
use unbound_spring::Spring;

use crate::motion::Look;
use crate::state::{Rect, State};
use crate::tile::Tile;

#[derive(Debug, Clone, Copy)]
enum Operation {
    Move { origin: Point<i32, Logical> },
    Resize,
}

#[derive(Debug, Clone, Copy)]
enum Placement {
    Free(Point<i32, Logical>),
    Maximized,
    Tiled(Tile),
}

#[derive(Debug, Clone, Copy)]
enum Undo {
    Move {
        placement: Placement,
        workspace: u64,
        restore: Option<Rect>,
    },
    Resize,
}

pub(crate) struct HeldWindow {
    surface: WlSurface,
    undo: Undo,
}

pub(crate) struct WindowGrab {
    start: GrabStartData<State>,
    held: Option<(WlSurface, Operation)>,
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

    fn undo_move(&self, surface: &WlSurface) -> Option<Undo> {
        let window = &self.windows[self.window_index(surface)?];
        let placement = match window.tile {
            Some((tile, _)) => Placement::Tiled(tile),
            None if window.has_state(xdg_toplevel::State::Maximized) => Placement::Maximized,
            None => Placement::Free(window.location),
        };
        Some(Undo::Move {
            placement,
            workspace: window.workspace,
            restore: window.restore,
        })
    }

    pub(crate) fn request_move(&mut self, surface: &WlSurface, seat: &WlSeat, serial: Serial) {
        let Some(start) = self.grab_start(seat, serial, surface) else {
            return;
        };
        let Some(undo) = self.undo_move(surface) else {
            return;
        };
        let Some(origin) = self.begin_move(surface, whole(start.location)) else {
            return;
        };
        self.start_grab(start, surface, Operation::Move { origin }, undo, serial);
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
            self.start_grab(start, surface, Operation::Resize, Undo::Resize, serial);
        }
    }

    pub(crate) fn move_from_title_bar(&mut self, surface: &WlSurface, button: u32) {
        let location = self.pointer_location;
        let Some(undo) = self.undo_move(surface) else {
            return;
        };
        let Some(origin) = self.begin_move(surface, whole(location)) else {
            return;
        };
        let start = GrabStartData {
            focus: None,
            button,
            location,
        };
        let serial = SERIAL_COUNTER.next_serial();
        self.start_grab(start, surface, Operation::Move { origin }, undo, serial);
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
        self.start_grab(start, surface, Operation::Resize, Undo::Resize, serial);
    }

    fn start_grab(
        &mut self,
        start: GrabStartData<State>,
        surface: &WlSurface,
        operation: Operation,
        undo: Undo,
        serial: Serial,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let grab = WindowGrab {
            start,
            held: Some((surface.clone(), operation)),
        };
        pointer.set_grab(self, grab, serial, Focus::Clear);
        self.held_window = Some(HeldWindow {
            surface: surface.clone(),
            undo,
        });
    }

    pub(crate) fn holds_cancellable_grab(&self) -> bool {
        self.held_window.is_some() || self.drag.is_some()
    }

    pub(crate) fn cancel_grab(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some(start) = pointer.grab_start_data() else {
            return;
        };
        let time = Clock::<Monotonic>::new().now().as_millis();
        if self.drag.is_some() {
            pointer.motion(
                self,
                None,
                &MotionEvent {
                    location: self.pointer_location,
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
        }
        if let Some(held) = self.held_window.take() {
            self.undo(&held.surface, held.undo);
        }
        let settling = WindowGrab { start, held: None };
        pointer.set_grab(self, settling, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn undo(&mut self, surface: &WlSurface, undo: Undo) {
        match undo {
            Undo::Resize => {
                self.resize_to(surface, Point::default());
                self.end_resize(surface);
            }
            Undo::Move {
                placement,
                workspace,
                restore,
            } => {
                self.cancel_snap();
                self.put_back(surface, placement, workspace, restore);
            }
        }
    }

    fn put_back(
        &mut self,
        surface: &WlSurface,
        placement: Placement,
        workspace: u64,
        restore: Option<Rect>,
    ) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let left = self.windows[index].workspace;
        if left != workspace {
            self.windows[index].workspace = workspace;
            self.window_left_workspace(left);
            self.sync_surface_outputs();
            self.announce_workspaces();
        }
        let Some(index) = self.window_index(surface) else {
            return;
        };
        match placement {
            Placement::Free(location) => {
                let from = self.windows[index].geometry().map(|rect| {
                    Look::shown(Rectangle::new(rect.location.to_f64(), rect.size.to_f64()))
                });
                let window = &mut self.windows[index];
                window.location = location;
                window.pending_location = None;
                window.motion = None;
                self.animate_window(index, Spring::STANDARD, from);
            }
            Placement::Maximized => self.maximize(surface),
            Placement::Tiled(tile) => self.tile_window(index, tile),
        }
        if let Some(index) = self.window_index(surface) {
            self.windows[index].restore = restore;
        }
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
        match &self.held {
            Some((surface, Operation::Move { origin })) => {
                data.move_window_to(surface, *origin + delta);
                data.track_snap(surface, event.location, event.time);
            }
            Some((surface, Operation::Resize)) => data.resize_to(surface, delta),
            None => {}
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
        let Some((surface, operation)) = &self.held else {
            return;
        };
        if data.held_window.take().is_none() {
            return;
        }
        match operation {
            Operation::Resize => data.end_resize(surface),
            Operation::Move { .. } => data.finish_move(surface),
        }
    }
}
