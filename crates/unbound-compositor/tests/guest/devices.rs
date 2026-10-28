use smithay::reexports::drm::control::{Device as ControlDevice, connector};
use std::path::Path;

use crate::card::{Card, card_with_driver};

#[test]
#[ignore = "needs the ci/vm guest"]
fn card0_is_a_modesetting_device() {
    let card = Card::open(Path::new("/dev/dri/card0")).expect("open /dev/dri/card0");
    let resources = card.resource_handles().expect("KMS resources");
    assert!(!resources.crtcs().is_empty());
    assert!(!resources.connectors().is_empty());
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn vkms_has_a_connected_output_writeback_and_every_plane_type() {
    let card = card_with_driver("vkms")
        .with_writeback_connectors()
        .expect("atomic modesetting with writeback connectors");
    let resources = card.resource_handles().expect("KMS resources");
    assert_eq!(resources.crtcs().len(), 1);

    let connectors: Vec<connector::Info> = resources
        .connectors()
        .iter()
        .map(|&handle| card.get_connector(handle, true).expect("connector"))
        .collect();
    let virtual_output = connectors
        .iter()
        .find(|info| info.interface() == connector::Interface::Virtual)
        .expect("a virtual connector");
    assert_eq!(virtual_output.state(), connector::State::Connected);
    assert!(!virtual_output.modes().is_empty());
    assert!(
        connectors
            .iter()
            .any(|info| info.interface() == connector::Interface::Writeback)
    );

    let mut plane_types: Vec<String> = card
        .plane_handles()
        .expect("planes")
        .into_iter()
        .map(|plane| card.plane_type(plane))
        .collect();
    plane_types.sort();
    plane_types.dedup();
    assert_eq!(plane_types, ["Cursor", "Overlay", "Primary"]);
}

#[test]
#[ignore = "needs the ci/vm guest"]
fn virtio_gpu_is_present_beside_vkms() {
    let card = card_with_driver("virtio_gpu");
    assert!(
        !card
            .resource_handles()
            .expect("KMS resources")
            .crtcs()
            .is_empty()
    );
}
