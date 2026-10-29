use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::{
    self, WpFractionalScaleV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

use crate::window::State;

const SCALE_DENOMINATOR: f64 = 120.0;

pub(crate) struct SurfaceScale {
    viewport: WpViewport,
    _fractional: WpFractionalScaleV1,
}

#[derive(Default)]
pub(crate) struct Scaling {
    pub(crate) fractional_manager: Option<WpFractionalScaleManagerV1>,
    pub(crate) viewporter: Option<WpViewporter>,
    preferred: Option<f64>,
}

impl Scaling {
    pub(crate) fn follow(
        &self,
        surface: &WlSurface,
        handle: &QueueHandle<State>,
    ) -> Option<SurfaceScale> {
        let (manager, viewporter) = (self.fractional_manager.as_ref()?, self.viewporter.as_ref()?);
        Some(SurfaceScale {
            viewport: viewporter.get_viewport(surface, handle, ()),
            _fractional: manager.get_fractional_scale(surface, handle, ()),
        })
    }

    pub(crate) fn scale(&self) -> f64 {
        self.preferred.unwrap_or(1.0)
    }
}

pub(crate) fn pixels(points: u32, scale: f64) -> u32 {
    (f64::from(points) * scale).round().max(1.0) as u32
}

impl SurfaceScale {
    pub(crate) fn show_at(&self, points: (u32, u32)) {
        let clamp = |value: u32| i32::try_from(value.max(1)).unwrap_or(i32::MAX);
        self.viewport
            .set_destination(clamp(points.0), clamp(points.1));
    }
}

impl Dispatch<WpFractionalScaleV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.scaling.preferred = Some(f64::from(scale) / SCALE_DENOMINATOR);
        }
    }
}

delegate_noop!(State: WpFractionalScaleManagerV1);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpViewport);
