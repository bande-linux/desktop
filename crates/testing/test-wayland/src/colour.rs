use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::color_management::v1::client::wp_color_management_surface_v1::WpColorManagementSurfaceV1;
use wayland_protocols::wp::color_management::v1::client::wp_color_manager_v1::{
    self, Primaries, RenderIntent, TransferFunction, WpColorManagerV1,
};
use wayland_protocols::wp::color_management::v1::client::wp_image_description_creator_params_v1::WpImageDescriptionCreatorParamsV1;
use wayland_protocols::wp::color_management::v1::client::wp_image_description_v1::{
    self, WpImageDescriptionV1,
};

use crate::{Compositor, Globals, TestClient};

const MANAGER: &str = "wp_color_manager_v1";

impl TestClient {
    pub fn offered_colour_management(&self) -> bool {
        self.globals
            .advertised
            .iter()
            .any(|(_, interface)| interface == MANAGER)
    }

    pub fn describe_as_pq(
        &mut self,
        server: &mut Compositor,
        index: usize,
        max_cll: u32,
    ) -> WpColorManagementSurfaceV1 {
        let qh = self.queue.handle();
        let manager: WpColorManagerV1 = self.bind(MANAGER, 1);
        let creator = manager.create_parametric_creator(&qh, ());
        creator.set_tf_named(TransferFunction::St2084Pq);
        creator.set_primaries_named(Primaries::Bt2020);
        creator.set_max_cll(max_cll);
        let description = creator.create(&qh, ());
        self.globals.descriptions_ready = 0;
        self.exchange(server);
        assert_eq!(self.globals.descriptions_ready, 1, "a ready description");
        let managed = manager.get_surface(&self.windows[index].0, &qh, ());
        managed.set_image_description(&description, RenderIntent::Perceptual);
        managed
    }
}

impl Dispatch<WpColorManagerV1, ()> for Globals {
    fn event(
        _: &mut Self,
        _: &WpColorManagerV1,
        _: wp_color_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WpImageDescriptionV1, ()> for Globals {
    fn event(
        state: &mut Self,
        _: &WpImageDescriptionV1,
        event: wp_image_description_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wp_image_description_v1::Event::Ready { .. } = event {
            state.descriptions_ready += 1;
        }
    }
}

delegate_noop!(Globals: ignore WpColorManagementSurfaceV1);
delegate_noop!(Globals: WpImageDescriptionCreatorParamsV1);
