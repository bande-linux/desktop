use std::sync::Mutex;

use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child};
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

use crate::clipboard::{Purpose, TEXT_TYPES, Transfer, own_type, pasted, send};
use crate::window::{State, Window, WindowError};

#[derive(Default)]
pub(crate) struct Primary {
    pub(crate) manager: Option<ZwpPrimarySelectionDeviceManagerV1>,
    device: Option<ZwpPrimarySelectionDeviceV1>,
    selection: Option<ZwpPrimarySelectionOfferV1>,
    selected: Option<(ZwpPrimarySelectionSourceV1, String)>,
}

#[derive(Default)]
pub(crate) struct PrimaryOffer {
    types: Mutex<Vec<String>>,
}

fn offered_types(offer: &ZwpPrimarySelectionOfferV1) -> Vec<String> {
    offer
        .data::<PrimaryOffer>()
        .and_then(|state| state.types.lock().ok().map(|types| types.clone()))
        .unwrap_or_default()
}

impl State {
    pub(crate) fn join_primary_selection(&mut self, handle: &QueueHandle<Self>) {
        let primary = &mut self.clipboard.primary;
        if let (Some(manager), Some(seat)) = (&primary.manager, &self.seat) {
            primary.device = Some(manager.get_device(seat, handle, ()));
        }
    }
}

impl Window {
    pub fn select_primary(&mut self, text: &str) {
        let primary = &mut self.state.clipboard.primary;
        let (Some(manager), Some(device), Some(serial)) =
            (&primary.manager, &primary.device, self.state.input.serial)
        else {
            return;
        };
        let source = manager.create_source(&self.queue.handle(), ());
        for mime in TEXT_TYPES {
            source.offer(mime.to_owned());
        }
        source.offer(own_type());
        device.set_selection(Some(&source), serial);
        if let Some((previous, _)) = primary.selected.replace((source, text.to_owned())) {
            previous.destroy();
        }
    }

    pub fn paste_primary(&mut self) -> Result<(), WindowError> {
        let primary = &self.state.clipboard.primary;
        let Some(offer) = primary.selection.clone() else {
            return Ok(());
        };
        let types = offered_types(&offer);
        if types.contains(&own_type())
            && let Some((_, text)) = &primary.selected
        {
            let event = pasted(Transfer::text(text), false);
            self.state.events.push(event);
            return Ok(());
        }
        let Some(mime) = TEXT_TYPES
            .into_iter()
            .find(|mime| types.iter().any(|offered| offered == mime))
        else {
            return Ok(());
        };
        self.read(
            mime.to_owned(),
            Purpose::Paste { transfer: false },
            |mime, fd| offer.receive(mime, fd),
        )
    }
}

impl Dispatch<ZwpPrimarySelectionDeviceV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_device_v1::Event::Selection { id } = event
            && let Some(previous) = std::mem::replace(&mut state.clipboard.primary.selection, id)
        {
            previous.destroy();
        }
    }

    event_created_child!(State, ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (ZwpPrimarySelectionOfferV1, PrimaryOffer::default()),
    ]);
}

impl Dispatch<ZwpPrimarySelectionOfferV1, PrimaryOffer> for State {
    fn event(
        _: &mut Self,
        _: &ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        offer: &PrimaryOffer,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event
            && let Ok(mut types) = offer.types.lock()
        {
            types.push(mime_type);
        }
    }
}

impl Dispatch<ZwpPrimarySelectionSourceV1, ()> for State {
    fn event(
        state: &mut Self,
        source: &ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let primary = &mut state.clipboard.primary;
        let ours = primary
            .selected
            .as_ref()
            .filter(|(selected, _)| selected == source);
        match event {
            zwp_primary_selection_source_v1::Event::Send { fd, .. } => {
                if let Some((_, text)) = ours {
                    send(fd, text.as_bytes());
                }
            }
            zwp_primary_selection_source_v1::Event::Cancelled => {
                if ours.is_some() {
                    primary.selected = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore ZwpPrimarySelectionDeviceManagerV1);
