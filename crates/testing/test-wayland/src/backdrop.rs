use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::{
    self, ExtBackgroundEffectManagerV1,
};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

use crate::{Compositor, Globals, TestClient};

impl TestClient {
    pub fn blur_behind(
        &mut self,
        server: &mut Compositor,
        index: usize,
        (width, height): (i32, i32),
    ) {
        let qh = self.queue.handle();
        let manager: ExtBackgroundEffectManagerV1 =
            self.bind("ext_background_effect_manager_v1", 1);
        let surface = self.windows[index].0.clone();
        let effect = manager.get_background_effect(&surface, &qh, ());
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let region = compositor.create_region(&qh, ());
        region.add(0, 0, width, height);
        effect.set_blur_region(Some(&region));
        region.destroy();
        surface.commit();
        self.exchange(server);
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Globals {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        _: ext_background_effect_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Globals: ignore ExtBackgroundEffectSurfaceV1);
delegate_noop!(Globals: WlRegion);
