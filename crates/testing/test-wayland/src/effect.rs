use wayland_client::delegate_noop;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

use crate::{Globals, Headless, TestClient};

delegate_noop!(Globals: ignore ExtBackgroundEffectManagerV1);
delegate_noop!(Globals: ExtBackgroundEffectSurfaceV1);
delegate_noop!(Globals: WlRegion);

impl TestClient {
    pub fn blur_behind(&mut self, server: &mut Headless, index: usize, rect: (i32, i32, i32, i32)) {
        let surface = self.windows[index].0.clone();
        self.blur_behind_surface(server, &surface, rect);
    }

    pub fn blur_behind_layer(
        &mut self,
        server: &mut Headless,
        index: usize,
        rect: (i32, i32, i32, i32),
    ) {
        let surface = self.globals.shell.layers[index].0.clone();
        self.blur_behind_surface(server, &surface, rect);
    }

    fn blur_behind_surface(
        &mut self,
        server: &mut Headless,
        surface: &WlSurface,
        (x, y, width, height): (i32, i32, i32, i32),
    ) {
        let manager: ExtBackgroundEffectManagerV1 =
            self.bind("ext_background_effect_manager_v1", 1);
        let handle = self.queue.handle();
        let effect = manager.get_background_effect(surface, &handle, ());
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let region = compositor.create_region(&handle, ());
        region.add(x, y, width, height);
        effect.set_blur_region(Some(&region));
        region.destroy();
        surface.commit();
        self.exchange(server);
    }
}
