use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

use smithay::reexports::drm::control::{Device as ControlDevice, connector, plane};
use smithay::reexports::drm::{ClientCapability, Device};

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

impl Card {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Card(OpenOptions::new().read(true).write(true).open(path)?))
    }

    fn with_writeback_connectors(self) -> io::Result<Self> {
        self.set_client_capability(ClientCapability::Atomic, true)?;
        self.set_client_capability(ClientCapability::WritebackConnectors, true)?;
        Ok(self)
    }

    fn driver(&self) -> String {
        let driver = self.get_driver().expect("DRM driver version");
        driver.name().to_string_lossy().into_owned()
    }

    fn plane_type(&self, plane: plane::Handle) -> String {
        let properties = self.get_properties(plane).expect("plane properties");
        properties
            .iter()
            .find_map(|(&handle, &raw)| {
                let info = self.get_property(handle).ok()?;
                if info.name().to_bytes() != b"type" {
                    return None;
                }
                let value_type = info.value_type();
                let value = value_type.convert_value(raw);
                value
                    .as_enum()
                    .map(|value| value.name().to_string_lossy().into_owned())
            })
            .expect("every plane has a type")
    }
}

fn cards() -> Vec<PathBuf> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/dev/dri")
        .expect("/dev/dri")
        .map(|entry| entry.expect("/dev/dri entry").path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("card"))
        })
        .collect();
    cards.sort();
    cards
}

fn card_with_driver(name: &str) -> Card {
    cards()
        .iter()
        .map(|path| Card::open(path).expect("open DRM card"))
        .find(|card| card.driver() == name)
        .unwrap_or_else(|| panic!("no DRM card driven by {name}"))
}

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
