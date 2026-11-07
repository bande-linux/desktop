use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_exported_v2::{self, ZxdgExportedV2};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_exporter_v2::ZxdgExporterV2;
use wayland_protocols::xdg::foreign::zv2::client::zxdg_imported_v2::ZxdgImportedV2;
use wayland_protocols::xdg::foreign::zv2::client::zxdg_importer_v2::ZxdgImporterV2;

use bande_compositor::Compositor;

use crate::{Globals, ShmImage, TestClient};

impl TestClient {
    pub fn export_window(&mut self, server: &mut Compositor, index: usize) -> String {
        let exporter: ZxdgExporterV2 = self.bind("zxdg_exporter_v2", 1);
        exporter.export_toplevel(&self.windows[index].0, &self.queue.handle(), ());
        self.exchange(server);
        self.globals
            .exported_handle
            .take()
            .expect("the compositor names the exported window")
    }

    pub fn show_window_attached_to(
        &mut self,
        server: &mut Compositor,
        image: &ShmImage,
        handle: &str,
    ) {
        let importer: ZxdgImporterV2 = self.bind("zxdg_importer_v2", 1);
        let imported = importer.import_toplevel(handle.to_owned(), &self.queue.handle(), ());
        self.show_window_with(server, image, |surface| imported.set_parent_of(surface));
    }
}

impl Dispatch<ZxdgExportedV2, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZxdgExportedV2,
        event: zxdg_exported_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_exported_v2::Event::Handle { handle } = event {
            globals.exported_handle = Some(handle);
        }
    }
}

delegate_noop!(Globals: ignore ZxdgExporterV2);
delegate_noop!(Globals: ignore ZxdgImporterV2);
delegate_noop!(Globals: ignore ZxdgImportedV2);
