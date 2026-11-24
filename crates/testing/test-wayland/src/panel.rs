use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{
    Layer, ZwlrLayerShellV1,
};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, ZwlrLayerSurfaceV1,
};

use crate::{Compositor, Globals, ShmImage, TestClient};

impl TestClient {
    pub fn show_blurred_panel(&mut self, server: &mut Compositor, image: &ShmImage) {
        let qh = self.queue.handle();
        let shell: ZwlrLayerShellV1 = self.bind("zwlr_layer_shell_v1", 1);
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        let panel =
            shell.get_layer_surface(&surface, None, Layer::Top, "panel".to_owned(), &qh, ());
        panel.set_size(image.width.unsigned_abs(), image.height.unsigned_abs());
        panel.set_anchor(Anchor::Top);
        surface.commit();
        self.exchange(server);
        self.blur_surface(&surface, (image.width, image.height));
        self.attach(&surface, image, (0, 0, image.width, image.height));
        self.exchange(server);
    }

    pub fn make_full_screen(&mut self, server: &mut Compositor, index: usize) {
        self.windows[index].2.set_fullscreen(None);
        self.exchange(server);
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Globals {
    fn event(
        _: &mut Self,
        panel: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            panel.ack_configure(serial);
        }
    }
}

delegate_noop!(Globals: ZwlrLayerShellV1);
