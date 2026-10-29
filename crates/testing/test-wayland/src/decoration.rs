use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::decoration::zv1::client::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1;
pub use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1::{
    self, ZxdgToplevelDecorationV1,
};

use crate::{Globals, Headless, Memory, ShmImage, TestClient};

impl TestClient {
    pub fn show_decorated_window(
        &mut self,
        server: &mut Headless,
        image: &ShmImage,
        requested: Option<DecorationMode>,
    ) {
        let manager: ZxdgDecorationManagerV1 = self.bind("zxdg_decoration_manager_v1", 1);
        self.map_window(server, image, Memory::Shm, |client, _, toplevel| {
            let decoration = manager.get_toplevel_decoration(toplevel, &client.queue.handle(), ());
            if let Some(mode) = requested {
                decoration.set_mode(mode);
            }
        });
    }

    pub fn decoration_modes(&self) -> &[DecorationMode] {
        &self.globals.decoration_modes
    }
}

impl Dispatch<ZxdgToplevelDecorationV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure {
            mode: WEnum::Value(mode),
        } = event
        {
            globals.decoration_modes.push(mode);
        }
    }
}

delegate_noop!(Globals: ignore ZxdgDecorationManagerV1);
