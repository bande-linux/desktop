use wayland_client::protocol::wl_shm::Format;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::ZwlrLayerShellV1;
pub use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::Anchor;
use wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    self, ZwlrLayerSurfaceV1,
};

use crate::{Globals, Headless, Memory, ShmImage, TestClient};

#[derive(Default)]
pub(crate) struct Shell {
    layer_size: Option<(u32, u32)>,
    token: Option<String>,
    preferred_scale: Option<u32>,
    layers: Vec<(WlSurface, ZwlrLayerSurfaceV1)>,
}

pub struct LayerRequest {
    pub layer: Layer,
    pub anchor: Anchor,
    pub size: (u32, u32),
    pub exclusive_zone: i32,
    pub color: [u8; 4],
}

impl TestClient {
    pub fn show_layer(&mut self, server: &mut Headless, request: &LayerRequest) -> (u32, u32) {
        let qh = self.queue.handle();
        let shell: ZwlrLayerShellV1 = self.bind("zwlr_layer_shell_v1", 4);
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        let layer_surface = shell.get_layer_surface(
            &surface,
            None,
            request.layer,
            "unbound-test".into(),
            &qh,
            (),
        );
        layer_surface.set_size(request.size.0, request.size.1);
        layer_surface.set_anchor(request.anchor);
        layer_surface.set_exclusive_zone(request.exclusive_zone);
        self.globals.shell.layer_size = None;
        surface.commit();
        self.exchange(server);
        let (width, height) = self
            .globals
            .shell
            .layer_size
            .expect("the layer surface is configured");
        let pixels = request.color.repeat((width * height) as usize);
        let buffer = self.shm_buffer(&ShmImage {
            width: width as i32,
            height: height as i32,
            stride: (width * 4) as i32,
            format: Format::Argb8888,
            bytes: &pixels,
        });
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.commit();
        self.exchange(server);
        self.globals.shell.layers.push((surface, layer_surface));
        (width, height)
    }

    pub fn activation_token(&mut self, server: &mut Headless) -> String {
        let activation: XdgActivationV1 = self.bind("xdg_activation_v1", 1);
        let token = activation.get_activation_token(&self.queue.handle(), ());
        if let Some((surface, _, _)) = self.windows.last() {
            token.set_surface(surface);
        }
        self.globals.shell.token = None;
        token.commit();
        self.exchange(server);
        self.globals
            .shell
            .token
            .take()
            .expect("an activation token")
    }

    pub fn activate(&mut self, server: &mut Headless, index: usize, token: &str) {
        let activation: XdgActivationV1 = self.bind("xdg_activation_v1", 1);
        activation.activate(token.to_owned(), &self.windows[index].0);
        self.exchange(server);
    }

    pub fn show_window_activated(&mut self, server: &mut Headless, image: &ShmImage, token: &str) {
        self.map_window(server, image, Memory::Shm, |client, surface| {
            let activation: XdgActivationV1 = client.bind("xdg_activation_v1", 1);
            activation.activate(token.to_owned(), surface);
        });
    }

    pub fn show_window_scaled(
        &mut self,
        server: &mut Headless,
        image: &ShmImage,
        destination: (i32, i32),
    ) {
        self.map_window(server, image, Memory::Shm, |client, surface| {
            let viewporter: WpViewporter = client.bind("wp_viewporter", 1);
            let viewport = viewporter.get_viewport(surface, &client.queue.handle(), ());
            viewport.set_destination(destination.0, destination.1);
        });
    }

    pub fn preferred_scale(&mut self, server: &mut Headless, index: usize) -> Option<u32> {
        let manager: WpFractionalScaleManagerV1 = self.bind("wp_fractional_scale_manager_v1", 1);
        manager.get_fractional_scale(&self.windows[index].0, &self.queue.handle(), ());
        self.exchange(server);
        self.globals.shell.preferred_scale
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer_surface.ack_configure(serial);
            globals.shell.layer_size = Some((width, height));
        }
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            globals.shell.token = Some(token);
        }
    }
}

impl Dispatch<WpFractionalScaleV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            globals.shell.preferred_scale = Some(scale);
        }
    }
}

delegate_noop!(Globals: ignore ZwlrLayerShellV1);
delegate_noop!(Globals: ignore XdgActivationV1);
delegate_noop!(Globals: ignore WpViewporter);
delegate_noop!(Globals: ignore WpViewport);
delegate_noop!(Globals: ignore WpFractionalScaleManagerV1);
