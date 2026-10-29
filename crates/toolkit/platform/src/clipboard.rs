use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Mutex;

use rustix::pipe::{PipeFlags, pipe_with};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};

use crate::window::{State, Window, WindowError, WindowEvent};

const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

fn own_type() -> String {
    format!("application/x-unbound-toolkit-{}", std::process::id())
}

struct Paste {
    pipe: File,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Clipboard {
    device: Option<WlDataDevice>,
    selection: Option<WlDataOffer>,
    copied: Option<(WlDataSource, String)>,
    pastes: Vec<Paste>,
}

type OfferTypes = Mutex<Vec<String>>;

fn offered_types(offer: &WlDataOffer) -> Vec<String> {
    offer
        .data::<OfferTypes>()
        .and_then(|types| types.lock().ok().map(|types| types.clone()))
        .unwrap_or_default()
}

impl State {
    pub(crate) fn join_data_device(&mut self, handle: &QueueHandle<Self>) {
        if let (Some(manager), Some(seat)) = (&self.data_device_manager, &self.seat) {
            self.clipboard.device = Some(manager.get_data_device(seat, handle, ()));
        }
    }
}

impl Window {
    pub fn copy(&mut self, text: &str) {
        let (Some(manager), Some(device), Some(serial)) = (
            &self.state.data_device_manager,
            &self.state.clipboard.device,
            self.state.input.serial,
        ) else {
            return;
        };
        let source = manager.create_data_source(&self.queue.handle(), ());
        for mime in TEXT_TYPES {
            source.offer(mime.to_owned());
        }
        source.offer(own_type());
        device.set_selection(Some(&source), serial);
        if let Some((previous, _)) = self
            .state
            .clipboard
            .copied
            .replace((source, text.to_owned()))
        {
            previous.destroy();
        }
    }

    pub fn paste(&mut self) -> Result<(), WindowError> {
        let Some(offer) = &self.state.clipboard.selection else {
            return Ok(());
        };
        let types = offered_types(offer);
        if types.contains(&own_type())
            && let Some((_, text)) = &self.state.clipboard.copied
        {
            self.state.events.push(WindowEvent::Pasted(text.clone()));
            return Ok(());
        }
        let Some(mime) = TEXT_TYPES
            .iter()
            .find(|mime| types.iter().any(|offered| offered == *mime))
        else {
            return Ok(());
        };
        let (reader, writer) =
            pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK).map_err(io::Error::from)?;
        offer.receive((*mime).to_owned(), writer.as_fd());
        drop(writer);
        self.flush()?;
        self.state.clipboard.pastes.push(Paste {
            pipe: File::from(reader),
            bytes: Vec::new(),
        });
        Ok(())
    }

    pub(crate) fn paste_fds(&self) -> Vec<BorrowedFd<'_>> {
        self.state
            .clipboard
            .pastes
            .iter()
            .map(|paste| paste.pipe.as_fd())
            .collect()
    }

    pub(crate) fn read_paste(&mut self, index: usize) {
        let Some(paste) = self.state.clipboard.pastes.get_mut(index) else {
            return;
        };
        let mut chunk = [0u8; 4096];
        let finished = loop {
            match paste.pipe.read(&mut chunk) {
                Ok(0) => break true,
                Ok(read) => paste.bytes.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break false,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break true,
            }
        };
        if finished {
            let paste = self.state.clipboard.pastes.remove(index);
            let text = String::from_utf8_lossy(&paste.bytes).into_owned();
            self.state.events.push(WindowEvent::Pasted(text));
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_device::Event::Selection { id } = event
            && let Some(previous) = std::mem::replace(&mut state.clipboard.selection, id)
        {
            previous.destroy();
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferTypes::default()),
    ]);
}

impl Dispatch<WlDataOffer, OfferTypes> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        types: &OfferTypes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Ok(mut types) = types.lock()
        {
            types.push(mime_type);
        }
    }
}

impl Dispatch<WlDataSource, ()> for State {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { fd, .. } => {
                if let Some((_, text)) = state
                    .clipboard
                    .copied
                    .as_ref()
                    .filter(|(copied, _)| copied == source)
                {
                    send(fd, text);
                }
            }
            wl_data_source::Event::Cancelled => {
                if state
                    .clipboard
                    .copied
                    .as_ref()
                    .is_some_and(|(copied, _)| copied == source)
                {
                    state.clipboard.copied = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

fn send(fd: OwnedFd, text: &str) {
    let mut pipe = File::from(fd);
    let _ = pipe.write_all(text.as_bytes());
}
