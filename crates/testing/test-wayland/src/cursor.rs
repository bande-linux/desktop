use wayland_client::delegate_noop;
pub use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape as CursorShape;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;

use crate::{Compositor, Globals, TestClient};

impl TestClient {
    pub fn set_cursor_shape(&mut self, server: &mut Compositor, shape: CursorShape) {
        let manager: WpCursorShapeManagerV1 = self.bind("wp_cursor_shape_manager_v1", 1);
        let pointer = self.globals.input.pointer.as_ref().expect("a pointer");
        let device = manager.get_pointer(pointer, &self.queue.handle(), ());
        device.set_shape(self.globals.input.pointer_enter_serial, shape);
        self.exchange(server);
    }
}

delegate_noop!(Globals: ignore WpCursorShapeManagerV1);
delegate_noop!(Globals: ignore WpCursorShapeDeviceV1);
