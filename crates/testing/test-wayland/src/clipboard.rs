use std::io::{PipeReader, Write};
use std::os::fd::AsFd;
use std::sync::Mutex;

use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1::{
    self, ExtDataControlDeviceV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::{
    self, ExtDataControlOfferV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_source_v1::{
    self, ExtDataControlSourceV1,
};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1;
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_v1::{
    self, ZwpPrimarySelectionDeviceV1,
};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1::{
    self, ZwpPrimarySelectionOfferV1,
};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_source_v1::{
    self, ZwpPrimarySelectionSourceV1,
};

use super::{Globals, TestClient};

pub const TEXT: &str = "text/plain;charset=utf-8";

type Mimes = Mutex<Vec<String>>;

#[derive(Default)]
pub(super) struct Clipboard {
    keyboard: Option<WlKeyboard>,
    focused: Option<WlSurface>,
    last_enter_serial: Option<u32>,
    data_device: Option<WlDataDevice>,
    primary_device: Option<ZwpPrimarySelectionDeviceV1>,
    control_device: Option<ExtDataControlDeviceV1>,
    selection: Option<WlDataOffer>,
    primary: Option<ZwpPrimarySelectionOfferV1>,
    control_selection: Option<ExtDataControlOfferV1>,
    sources: Vec<WlDataSource>,
    primary_sources: Vec<ZwpPrimarySelectionSourceV1>,
    control_sources: Vec<ExtDataControlSourceV1>,
    cancelled_sources: usize,
}

impl Clipboard {
    pub(crate) fn data_device(&self) -> &WlDataDevice {
        self.data_device.as_ref().expect("data device")
    }

    pub(crate) fn keep_source(&mut self, source: WlDataSource) {
        self.sources.push(source);
    }
}

fn mimes<P: Proxy>(offer: &P) -> Vec<String> {
    offer
        .data::<Mimes>()
        .map(|mimes| mimes.lock().expect("mimes").clone())
        .unwrap_or_default()
}

impl TestClient {
    pub fn join_seat(&mut self, server: &mut unbound_compositor::Headless) {
        let qh = self.queue.handle();
        let seat: WlSeat = self.bind("wl_seat", 7);
        let data_manager: WlDataDeviceManager = self.bind("wl_data_device_manager", 3);
        let primary_manager: ZwpPrimarySelectionDeviceManagerV1 =
            self.bind("zwp_primary_selection_device_manager_v1", 1);
        let clipboard = &mut self.globals.clipboard;
        clipboard.keyboard = Some(seat.get_keyboard(&qh, ()));
        clipboard.data_device = Some(data_manager.get_data_device(&seat, &qh, ()));
        clipboard.primary_device = Some(primary_manager.get_device(&seat, &qh, ()));
        self.exchange(server);
    }

    pub fn join_data_control(&mut self, server: &mut unbound_compositor::Headless) {
        let qh = self.queue.handle();
        let seat: WlSeat = self.bind("wl_seat", 7);
        let manager: ExtDataControlManagerV1 = self.bind("ext_data_control_manager_v1", 1);
        self.globals.clipboard.control_device = Some(manager.get_data_device(&seat, &qh, ()));
        self.exchange(server);
    }

    pub fn has_keyboard_focus(&self) -> bool {
        self.globals.clipboard.focused.is_some()
    }

    pub fn focused_window(&self) -> Option<usize> {
        let focused = self.globals.clipboard.focused.as_ref()?;
        self.windows
            .iter()
            .position(|(surface, _, _)| surface == focused)
    }

    pub fn copy(&mut self, mime: &str, payload: &[u8]) {
        let qh = self.queue.handle();
        let manager: WlDataDeviceManager = self.bind("wl_data_device_manager", 3);
        let source = manager.create_data_source(&qh, payload.to_vec());
        source.offer(mime.to_owned());
        let clipboard = &mut self.globals.clipboard;
        let serial = clipboard.last_enter_serial.expect("keyboard entered once");
        clipboard
            .data_device
            .as_ref()
            .expect("data device")
            .set_selection(Some(&source), serial);
        clipboard.sources.push(source);
    }

    pub fn copy_primary(&mut self, mime: &str, payload: &[u8]) {
        let qh = self.queue.handle();
        let manager: ZwpPrimarySelectionDeviceManagerV1 =
            self.bind("zwp_primary_selection_device_manager_v1", 1);
        let source = manager.create_source(&qh, payload.to_vec());
        source.offer(mime.to_owned());
        let clipboard = &mut self.globals.clipboard;
        let serial = clipboard.last_enter_serial.expect("keyboard entered once");
        clipboard
            .primary_device
            .as_ref()
            .expect("primary device")
            .set_selection(Some(&source), serial);
        clipboard.primary_sources.push(source);
    }

    pub fn control_copy(&mut self, mimes: &[String], payload: &[u8]) {
        let qh = self.queue.handle();
        let manager: ExtDataControlManagerV1 = self.bind("ext_data_control_manager_v1", 1);
        let source = manager.create_data_source(&qh, payload.to_vec());
        for mime in mimes {
            source.offer(mime.clone());
        }
        let clipboard = &mut self.globals.clipboard;
        clipboard
            .control_device
            .as_ref()
            .expect("data control device")
            .set_selection(Some(&source));
        clipboard.control_sources.push(source);
    }

    pub fn cancelled_sources(&self) -> usize {
        self.globals.clipboard.cancelled_sources
    }

    pub fn selection_mimes(&self) -> Option<Vec<String>> {
        self.globals.clipboard.selection.as_ref().map(mimes)
    }

    pub fn primary_mimes(&self) -> Option<Vec<String>> {
        self.globals.clipboard.primary.as_ref().map(mimes)
    }

    pub fn control_mimes(&self) -> Option<Vec<String>> {
        self.globals.clipboard.control_selection.as_ref().map(mimes)
    }

    pub fn request_paste(&mut self, mime: &str) -> PipeReader {
        let (reader, writer) = std::io::pipe().expect("pipe");
        let offer = self
            .globals
            .clipboard
            .selection
            .as_ref()
            .expect("selection");
        offer.receive(mime.to_owned(), writer.as_fd());
        reader
    }

    pub fn request_primary_paste(&mut self, mime: &str) -> PipeReader {
        let (reader, writer) = std::io::pipe().expect("pipe");
        let offer = self
            .globals
            .clipboard
            .primary
            .as_ref()
            .expect("primary selection");
        offer.receive(mime.to_owned(), writer.as_fd());
        reader
    }

    pub fn request_control_paste(&mut self, mime: &str) -> PipeReader {
        let (reader, writer) = std::io::pipe().expect("pipe");
        let offer = self
            .globals
            .clipboard
            .control_selection
            .as_ref()
            .expect("data control selection");
        offer.receive(mime.to_owned(), writer.as_fd());
        reader
    }
}

fn serve(payload: &[u8], fd: std::os::fd::OwnedFd) {
    let mut pipe = std::fs::File::from(fd);
    pipe.write_all(payload).expect("write selection");
}

impl Dispatch<WlKeyboard, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter {
                serial, surface, ..
            } => {
                globals.clipboard.focused = Some(surface);
                globals.clipboard.last_enter_serial = Some(serial);
            }
            wl_keyboard::Event::Leave { .. } => globals.clipboard.focused = None,
            _ => {}
        }
    }
}

impl Dispatch<WlDataDevice, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::Selection { id } => globals.clipboard.selection = id,
            wl_data_device::Event::Enter { serial, id, .. } => globals.drag_entered(serial, id),
            wl_data_device::Event::Drop => globals.drag_dropped(),
            wl_data_device::Event::Leave => globals.dnd.leaves += 1,
            _ => {}
        }
    }

    event_created_child!(Globals, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, Mimes::default()),
    ]);
}

impl Dispatch<WlDataOffer, Mimes> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        mimes: &Mimes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                mimes.lock().expect("mimes").push(mime_type);
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(action),
            } => globals.dnd.chosen.push(action),
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, Vec<u8>> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlDataSource,
        event: wl_data_source::Event,
        payload: &Vec<u8>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { fd, .. } => serve(payload, fd),
            wl_data_source::Event::Cancelled => globals.clipboard.cancelled_sources += 1,
            _ => {}
        }
    }
}

impl Dispatch<ZwpPrimarySelectionDeviceV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_device_v1::Event::Selection { id } = event {
            globals.clipboard.primary = id;
        }
    }

    event_created_child!(Globals, ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (ZwpPrimarySelectionOfferV1, Mimes::default()),
    ]);
}

impl Dispatch<ZwpPrimarySelectionOfferV1, Mimes> for Globals {
    fn event(
        _: &mut Self,
        _: &ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        mimes: &Mimes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event {
            mimes.lock().expect("mimes").push(mime_type);
        }
    }
}

impl Dispatch<ZwpPrimarySelectionSourceV1, Vec<u8>> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        payload: &Vec<u8>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_primary_selection_source_v1::Event::Send { fd, .. } => serve(payload, fd),
            zwp_primary_selection_source_v1::Event::Cancelled => {
                globals.clipboard.cancelled_sources += 1
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_device_v1::Event::Selection { id } = event {
            globals.clipboard.control_selection = id;
        }
    }

    event_created_child!(Globals, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, Mimes::default()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, Mimes> for Globals {
    fn event(
        _: &mut Self,
        _: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        mimes: &Mimes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            mimes.lock().expect("mimes").push(mime_type);
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, Vec<u8>> for Globals {
    fn event(
        _: &mut Self,
        _: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        payload: &Vec<u8>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_source_v1::Event::Send { fd, .. } = event {
            serve(payload, fd);
        }
    }
}

delegate_noop!(Globals: ignore WlSeat);
delegate_noop!(Globals: WlDataDeviceManager);
delegate_noop!(Globals: ZwpPrimarySelectionDeviceManagerV1);
delegate_noop!(Globals: ExtDataControlManagerV1);
