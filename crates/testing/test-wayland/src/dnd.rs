use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::clipboard::TEXT;
use crate::{Compositor, Globals, ShmImage, TestClient};

#[derive(Default)]
pub(crate) struct Dnd {
    pub(crate) accepts: bool,
    pub(crate) holds: bool,
    pub(crate) offer: Option<WlDataOffer>,
    pub(crate) chosen: Vec<DndAction>,
    pub(crate) drops: usize,
    pub(crate) leaves: usize,
    icon: Option<WlSurface>,
}

impl Globals {
    pub(crate) fn drag_entered(&mut self, serial: u32, offer: Option<WlDataOffer>) {
        if let Some(offer) = &offer
            && self.dnd.accepts
        {
            offer.accept(serial, Some(TEXT.to_owned()));
        }
        self.dnd.offer = offer;
        self.drag_moved();
    }

    pub(crate) fn drag_moved(&mut self) {
        if let Some(offer) = &self.dnd.offer
            && self.dnd.accepts
        {
            offer.set_actions(DndAction::Copy | DndAction::Move, DndAction::Move);
        }
    }

    pub(crate) fn drag_dropped(&mut self) {
        self.dnd.drops += 1;
        if let Some(offer) = self.dnd.offer.take()
            && self.dnd.accepts
            && !self.dnd.holds
        {
            offer.finish();
        }
    }
}

impl TestClient {
    pub fn accept_drops(&mut self, accepts: bool) {
        self.globals.dnd.accepts = accepts;
    }

    pub fn hold_drops(&mut self) {
        self.globals.dnd.holds = true;
    }

    pub fn start_drag(&mut self, server: &mut Compositor, icon: &ShmImage, offset: (i32, i32)) {
        let qh = self.queue.handle();
        let manager: WlDataDeviceManager = self.bind("wl_data_device_manager", 3);
        let source = manager.create_data_source(&qh, b"dragged".to_vec());
        source.offer(TEXT.to_owned());
        source.set_actions(DndAction::Copy | DndAction::Move);
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        surface.attach(Some(&self.shm_buffer(icon)), offset.0, offset.1);
        surface.damage_buffer(0, 0, icon.width, icon.height);
        surface.commit();
        let origin = &self.windows[0].0;
        self.globals.clipboard.data_device().start_drag(
            Some(&source),
            origin,
            Some(&surface),
            self.globals.input.button_serial,
        );
        self.globals.clipboard.keep_source(source);
        self.globals.dnd.icon = Some(surface);
        self.exchange(server);
    }

    pub fn drag_actions(&self) -> &[DndAction] {
        &self.globals.dnd.chosen
    }

    pub fn drops(&self) -> usize {
        self.globals.dnd.drops
    }

    pub fn drag_leaves(&self) -> usize {
        self.globals.dnd.leaves
    }
}
