use bande_protocols::client::bande_shell_v1::BandeShellV1;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::{self, ZwlrLayerShellV1};
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, Anchor, ZwlrLayerSurfaceV1,
};

use crate::{Compositor, Globals, ShmImage, TestClient};

pub(crate) struct Panel {
    surface: WlSurface,
    role: ZwlrLayerSurfaceV1,
    shell: BandeShellV1,
}

impl TestClient {
    pub fn show_panel(&mut self, server: &mut Compositor, image: &ShmImage, top: i32) {
        let qh = self.queue.handle();
        let layers: ZwlrLayerShellV1 = self.bind("zwlr_layer_shell_v1", 4);
        let shell: BandeShellV1 = self.bind("bande_shell_v1", 7);
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        let role = layers.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Overlay,
            "launcher".to_owned(),
            &qh,
            (),
        );
        role.set_size(image.width as u32, image.height as u32);
        role.set_anchor(Anchor::Top);
        role.set_margin(top, 0, 0, 0);
        shell.dismiss_on_press_outside(&surface);
        self.globals.configured = false;
        surface.commit();
        self.exchange(server);
        assert!(self.globals.configured, "initial configure");
        self.attach(&surface, image, (0, 0, image.width, image.height));
        self.exchange(server);
        self.panel = Some(Panel {
            surface,
            role,
            shell,
        });
    }

    pub fn close_panel(&mut self, server: &mut Compositor) {
        if let Some(panel) = self.panel.take() {
            panel.role.destroy();
            panel.surface.destroy();
            panel.shell.destroy();
        }
        self.exchange(server);
    }

    pub fn move_panel(&mut self, server: &mut Compositor) {
        let panel = self.panel.as_ref().expect("show_panel first");
        let input = &self.globals.input;
        panel.shell.move_layer(
            &panel.surface,
            input.seat.as_ref().expect("use_seat first"),
            input.button_serial,
        );
        self.exchange(server);
    }

    pub fn pressed_outside(&self) -> usize {
        self.globals.pressed_outside
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for Globals {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        role: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
            role.ack_configure(serial);
            globals.configured = true;
        }
    }
}
