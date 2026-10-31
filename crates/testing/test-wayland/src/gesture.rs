use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_hold_v1::{
    self, ZwpPointerGestureHoldV1,
};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_pinch_v1::{
    self, ZwpPointerGesturePinchV1,
};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gesture_swipe_v1::{
    self, ZwpPointerGestureSwipeV1,
};
use wayland_protocols::wp::pointer_gestures::zv1::client::zwp_pointer_gestures_v1::ZwpPointerGesturesV1;

use crate::{Compositor, Globals, TestClient};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClientGesture {
    Swipe { fingers: u32 },
    SwipeMoved { dx: f64, dy: f64 },
    SwipeEnded { cancelled: bool },
    Pinch { fingers: u32 },
    PinchMoved { scale: f64 },
    PinchEnded { cancelled: bool },
    Hold { fingers: u32 },
    HoldEnded { cancelled: bool },
}

delegate_noop!(Globals: ZwpPointerGesturesV1);

impl TestClient {
    pub fn use_gestures(&mut self, server: &mut Compositor) {
        let gestures: ZwpPointerGesturesV1 = self.bind("zwp_pointer_gestures_v1", 3);
        let pointer = self.globals.input.pointer.clone().expect("a pointer");
        let handle = self.queue.handle();
        gestures.get_swipe_gesture(&pointer, &handle, ());
        gestures.get_pinch_gesture(&pointer, &handle, ());
        gestures.get_hold_gesture(&pointer, &handle, ());
        self.exchange(server);
    }

    pub fn gestures(&self) -> &[ClientGesture] {
        &self.globals.gestures
    }
}

impl Dispatch<ZwpPointerGestureSwipeV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpPointerGestureSwipeV1,
        event: zwp_pointer_gesture_swipe_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        globals.gestures.extend(match event {
            zwp_pointer_gesture_swipe_v1::Event::Begin { fingers, .. } => {
                Some(ClientGesture::Swipe { fingers })
            }
            zwp_pointer_gesture_swipe_v1::Event::Update { dx, dy, .. } => {
                Some(ClientGesture::SwipeMoved { dx, dy })
            }
            zwp_pointer_gesture_swipe_v1::Event::End { cancelled, .. } => {
                Some(ClientGesture::SwipeEnded {
                    cancelled: cancelled != 0,
                })
            }
            _ => None,
        });
    }
}

impl Dispatch<ZwpPointerGesturePinchV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpPointerGesturePinchV1,
        event: zwp_pointer_gesture_pinch_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        globals.gestures.extend(match event {
            zwp_pointer_gesture_pinch_v1::Event::Begin { fingers, .. } => {
                Some(ClientGesture::Pinch { fingers })
            }
            zwp_pointer_gesture_pinch_v1::Event::Update { scale, .. } => {
                Some(ClientGesture::PinchMoved { scale })
            }
            zwp_pointer_gesture_pinch_v1::Event::End { cancelled, .. } => {
                Some(ClientGesture::PinchEnded {
                    cancelled: cancelled != 0,
                })
            }
            _ => None,
        });
    }
}

impl Dispatch<ZwpPointerGestureHoldV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpPointerGestureHoldV1,
        event: zwp_pointer_gesture_hold_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        globals.gestures.extend(match event {
            zwp_pointer_gesture_hold_v1::Event::Begin { fingers, .. } => {
                Some(ClientGesture::Hold { fingers })
            }
            zwp_pointer_gesture_hold_v1::Event::End { cancelled, .. } => {
                Some(ClientGesture::HoldEnded {
                    cancelled: cancelled != 0,
                })
            }
            _ => None,
        });
    }
}
