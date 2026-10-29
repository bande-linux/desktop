use std::sync::Mutex;

use smithay::backend::renderer::element::Id;
use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::ext_background_effect_manager_v1::{
    self, Capability, ExtBackgroundEffectManagerV1,
};
use smithay::reexports::wayland_protocols::ext::background_effect::v1::server::ext_background_effect_surface_v1::{
    self, ExtBackgroundEffectSurfaceV1,
};
use smithay::reexports::wayland_server::protocol::wl_region::WlRegion;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Rectangle};
use smithay::wayland::compositor::{RectangleKind, get_region_attributes, with_states};

use crate::state::State;

type Rects = Vec<Rectangle<i32, Logical>>;

#[derive(Default)]
struct BlurRegion {
    attached: bool,
    pending: Option<Rects>,
    current: Rects,
    ids: Vec<Id>,
}

fn with_region<T>(surface: &WlSurface, f: impl FnOnce(&mut BlurRegion) -> T) -> T {
    with_states(surface, |states| {
        states
            .data_map
            .insert_if_missing_threadsafe(|| Mutex::new(BlurRegion::default()));
        let mut region = states.data_map.get::<Mutex<BlurRegion>>().map(|region| {
            region
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        match region.as_deref_mut() {
            Some(region) => f(region),
            None => f(&mut BlurRegion::default()),
        }
    })
}

fn rects_of(region: &WlRegion) -> Rects {
    get_region_attributes(region)
        .rects
        .into_iter()
        .fold(Vec::new(), |rects, (kind, rect)| match kind {
            RectangleKind::Add => rects.into_iter().chain([rect]).collect(),
            RectangleKind::Subtract => Rectangle::subtract_rects_many(rects, [rect]),
        })
}

pub(crate) fn commit_blur(surface: &WlSurface) {
    with_region(surface, |region| {
        if let Some(pending) = region.pending.take() {
            region.current = pending;
        }
    });
}

pub(crate) fn blur_rects(surface: &WlSurface) -> Vec<(Id, Rectangle<i32, Logical>)> {
    with_region(surface, |region| {
        while region.ids.len() < region.current.len() {
            region.ids.push(Id::new());
        }
        region
            .ids
            .iter()
            .cloned()
            .zip(region.current.iter().copied())
            .collect()
    })
}

impl GlobalDispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ExtBackgroundEffectManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        manager.capabilities(Capability::Blur);
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        manager: &ExtBackgroundEffectManagerV1,
        request: ext_background_effect_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_background_effect_manager_v1::Request::GetBackgroundEffect { id, surface } =
            request
        {
            if with_region(&surface, |region| {
                std::mem::replace(&mut region.attached, true)
            }) {
                manager.post_error(
                    ext_background_effect_manager_v1::Error::BackgroundEffectExists,
                    "the surface already has a background effect",
                );
                return;
            }
            data_init.init(id, surface);
        }
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, WlSurface> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        effect: &ExtBackgroundEffectSurfaceV1,
        request: ext_background_effect_surface_v1::Request,
        surface: &WlSurface,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_background_effect_surface_v1::Request::SetBlurRegion { region } => {
                if !surface.is_alive() {
                    effect.post_error(
                        ext_background_effect_surface_v1::Error::SurfaceDestroyed,
                        "the surface has been destroyed",
                    );
                    return;
                }
                let rects = region.as_ref().map(rects_of).unwrap_or_default();
                with_region(surface, |blur| blur.pending = Some(rects));
            }
            ext_background_effect_surface_v1::Request::Destroy if surface.is_alive() => {
                with_region(surface, |blur| {
                    blur.attached = false;
                    blur.pending = Some(Vec::new());
                });
            }
            _ => {}
        }
    }
}
