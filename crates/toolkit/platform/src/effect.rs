use unbound_graphics::Rect;
use wayland_client::delegate_noop;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

use crate::window::{State, Window, WindowError};

#[derive(Default)]
pub(crate) struct BackgroundEffect {
    pub(crate) manager: Option<ExtBackgroundEffectManagerV1>,
    surface: Option<ExtBackgroundEffectSurfaceV1>,
}

impl Window {
    pub fn set_background_blur(&mut self, rects: &[Rect]) -> Result<(), WindowError> {
        let effect = &mut self.state.effect;
        let manager = effect
            .manager
            .as_ref()
            .ok_or(WindowError::Missing("ext_background_effect_manager_v1"))?;
        let compositor = self
            .state
            .compositor
            .as_ref()
            .ok_or(WindowError::Missing("wl_compositor"))?;
        let handle = self.queue.handle();
        let surface = effect
            .surface
            .get_or_insert_with(|| manager.get_background_effect(&self.surface, &handle, ()));
        let region = compositor.create_region(&handle, ());
        for rect in rects {
            region.add(
                rect.x.round() as i32,
                rect.y.round() as i32,
                rect.width.round() as i32,
                rect.height.round() as i32,
            );
        }
        surface.set_blur_region(Some(&region));
        region.destroy();
        self.request_redraw();
        Ok(())
    }
}

delegate_noop!(State: ignore ExtBackgroundEffectManagerV1);
delegate_noop!(State: ExtBackgroundEffectSurfaceV1);
