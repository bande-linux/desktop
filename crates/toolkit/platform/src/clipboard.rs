use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bande_graphics::{Point, Scene};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::pipe::{PipeFlags, pipe_with};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};

use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use crate::primary::Primary;
use crate::window::{ShmTarget, State, Target, Window, WindowError, WindowEvent};

pub(crate) const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];
const URI_LIST: &str = "text/uri-list";
const GNOME_FILES: &str = "x-special/gnome-copied-files";
const PNG: &str = "image/png";
const FINISH_WAIT: Duration = Duration::from_secs(2);

pub(crate) fn own_type() -> String {
    format!("application/x-bande-toolkit-{}", std::process::id())
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transfer {
    pub text: String,
    pub uris: Vec<String>,
    pub cut: bool,
    pub png: Vec<u8>,
}

impl Transfer {
    pub fn text(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            uris: Vec::new(),
            cut: false,
            png: Vec::new(),
        }
    }

    pub fn uris(uris: Vec<String>) -> Self {
        Self {
            text: uris.join("\n"),
            uris,
            cut: false,
            png: Vec::new(),
        }
    }

    pub fn cut(self) -> Self {
        Self { cut: true, ..self }
    }

    pub fn with_png(self, png: Vec<u8>) -> Self {
        Self { png, ..self }
    }

    fn offer(&self, source: &WlDataSource) {
        if !self.png.is_empty() {
            source.offer(PNG.to_owned());
        }
        if !self.uris.is_empty() {
            source.offer(GNOME_FILES.to_owned());
            source.offer(URI_LIST.to_owned());
        }
        for mime in TEXT_TYPES {
            source.offer(mime.to_owned());
        }
        source.offer(own_type());
    }

    fn bytes(&self, mime: &str) -> Vec<u8> {
        match mime {
            GNOME_FILES => {
                let verb = if self.cut { "cut" } else { "copy" };
                std::iter::once(verb)
                    .chain(self.uris.iter().map(String::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into_bytes()
            }
            URI_LIST => self
                .uris
                .iter()
                .map(|uri| format!("{uri}\r\n"))
                .collect::<String>()
                .into_bytes(),
            PNG => self.png.clone(),
            _ => self.text.clone().into_bytes(),
        }
    }

    fn is_empty(&self) -> bool {
        self.text.is_empty() && self.uris.is_empty() && self.png.is_empty()
    }

    fn parse(mime: &str, bytes: &[u8]) -> Self {
        let text = String::from_utf8_lossy(bytes).into_owned();
        match mime {
            GNOME_FILES => {
                let mut lines = text.lines();
                let cut = lines.next() == Some("cut");
                Self {
                    cut,
                    ..Self::uris(
                        lines
                            .filter(|line| !line.is_empty())
                            .map(str::to_owned)
                            .collect(),
                    )
                }
            }
            URI_LIST => Self::uris(
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_owned)
                    .collect(),
            ),
            _ => Self::text(&text),
        }
    }
}

pub use bande_widgets::{ClipboardRequest, DragAction};

fn wire(action: DragAction) -> DndAction {
    match action {
        DragAction::Copy => DndAction::Copy,
        DragAction::Move => DndAction::Move,
    }
}

fn from_wire(action: DndAction) -> Option<DragAction> {
    if action.contains(DndAction::Move) {
        Some(DragAction::Move)
    } else if action.contains(DndAction::Copy) {
        Some(DragAction::Copy)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct DragIcon {
    pub scene: Scene,
    pub hotspot: Point,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DragEvent {
    Entered {
        position: Point,
        uris: Vec<String>,
        text: bool,
    },
    Moved(Point),
    Left,
    Dropped {
        position: Point,
        transfer: Transfer,
        action: Option<DragAction>,
    },
    Finished(Option<DragAction>),
}

pub(crate) enum Purpose {
    Paste {
        transfer: bool,
    },
    Drop {
        offer: WlDataOffer,
        position: Point,
        action: Option<DragAction>,
    },
    Hover {
        offer: WlDataOffer,
    },
}

struct Reading {
    pipe: File,
    mime: String,
    bytes: Vec<u8>,
    purpose: Purpose,
}

struct Outgoing {
    source: WlDataSource,
    transfer: Transfer,
    action: Option<DragAction>,
    icon: Option<Icon>,
    dropped_at: Option<Instant>,
}

struct Icon {
    surface: WlSurface,
    viewport: Option<WpViewport>,
    target: ShmTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    Unread,
    Reading,
    Announced,
}

struct Incoming {
    offer: WlDataOffer,
    serial: u32,
    position: Point,
    accepted: Option<DragAction>,
    only: bool,
    arrival: Arrival,
}

#[derive(Default)]
pub(crate) struct Clipboard {
    device: Option<WlDataDevice>,
    selection: Option<WlDataOffer>,
    copied: Option<(WlDataSource, Transfer)>,
    dragged: Option<Outgoing>,
    incoming: Option<Incoming>,
    pending_drop: Option<Incoming>,
    readings: Vec<Reading>,
    pub(crate) primary: Primary,
}

#[derive(Default)]
pub(crate) struct OfferState {
    types: Mutex<Vec<String>>,
    action: Mutex<Option<DragAction>>,
}

fn offered_types(offer: &WlDataOffer) -> Vec<String> {
    offer
        .data::<OfferState>()
        .and_then(|state| state.types.lock().ok().map(|types| types.clone()))
        .unwrap_or_default()
}

fn offered_action(offer: &WlDataOffer) -> Option<DragAction> {
    offer
        .data::<OfferState>()
        .and_then(|state| state.action.lock().ok().and_then(|action| *action))
}

fn readable_type(types: &[String], uris: bool) -> Option<String> {
    let preferred = uris
        .then_some([GNOME_FILES, URI_LIST])
        .into_iter()
        .flatten()
        .chain(TEXT_TYPES);
    preferred
        .into_iter()
        .find(|mime| types.iter().any(|offered| offered == mime))
        .map(str::to_owned)
}

impl State {
    pub(crate) fn join_data_device(&mut self, handle: &QueueHandle<Self>) {
        if let (Some(manager), Some(seat)) = (&self.data_device_manager, &self.seat) {
            self.clipboard.device = Some(manager.get_data_device(seat, handle, ()));
        }
    }

    fn drag_event(&mut self, event: DragEvent) {
        self.events.push(WindowEvent::Drag(event));
    }
}

impl Window {
    pub fn serve_clipboard(&mut self, request: ClipboardRequest) -> Result<(), WindowError> {
        match request {
            ClipboardRequest::Copy(text) => self.copy(&text),
            ClipboardRequest::Paste => return self.paste(),
            ClipboardRequest::SelectPrimary(text) => self.select_primary(&text),
            ClipboardRequest::PastePrimary => return self.paste_primary(),
        }
        Ok(())
    }

    pub fn copy(&mut self, text: &str) {
        self.copy_transfer(Transfer::text(text));
    }

    pub fn copy_transfer(&mut self, transfer: Transfer) {
        let (Some(manager), Some(device), Some(serial)) = (
            &self.state.data_device_manager,
            &self.state.clipboard.device,
            self.state.input.serial,
        ) else {
            return;
        };
        let source = manager.create_data_source(&self.queue.handle(), ());
        transfer.offer(&source);
        device.set_selection(Some(&source), serial);
        if let Some((previous, _)) = self.state.clipboard.copied.replace((source, transfer)) {
            previous.destroy();
        }
    }

    pub fn clipboard_has_uris(&self) -> bool {
        self.state
            .clipboard
            .selection
            .as_ref()
            .is_some_and(|offer| offered_types(offer).iter().any(|mime| mime == URI_LIST))
    }

    pub fn paste(&mut self) -> Result<(), WindowError> {
        self.paste_as(false)
    }

    pub fn paste_transfer(&mut self) -> Result<(), WindowError> {
        self.paste_as(true)
    }

    fn paste_as(&mut self, transfer: bool) -> Result<(), WindowError> {
        let Some(offer) = self.state.clipboard.selection.clone() else {
            return Ok(());
        };
        let types = offered_types(&offer);
        if types.contains(&own_type())
            && let Some((_, copied)) = &self.state.clipboard.copied
        {
            let event = pasted(copied.clone(), transfer);
            self.state.events.push(event);
            return Ok(());
        }
        let Some(mime) = readable_type(&types, transfer) else {
            return Ok(());
        };
        self.read_offer(&offer, mime, Purpose::Paste { transfer })
    }

    fn read_offer(
        &mut self,
        offer: &WlDataOffer,
        mime: String,
        purpose: Purpose,
    ) -> Result<(), WindowError> {
        self.read(mime, purpose, |mime, fd| offer.receive(mime, fd))
    }

    pub(crate) fn read(
        &mut self,
        mime: String,
        purpose: Purpose,
        receive: impl FnOnce(String, BorrowedFd),
    ) -> Result<(), WindowError> {
        let (reader, writer) =
            pipe_with(PipeFlags::CLOEXEC | PipeFlags::NONBLOCK).map_err(io::Error::from)?;
        receive(mime.clone(), writer.as_fd());
        drop(writer);
        self.flush()?;
        self.state.clipboard.readings.push(Reading {
            pipe: File::from(reader),
            mime,
            bytes: Vec::new(),
            purpose,
        });
        Ok(())
    }

    pub fn start_drag(
        &mut self,
        transfer: Transfer,
        actions: &[DragAction],
        icon: Option<DragIcon>,
    ) -> Result<(), WindowError> {
        let (Some(manager), Some(device), Some(serial)) = (
            &self.state.data_device_manager,
            &self.state.clipboard.device,
            self.state.input.serial,
        ) else {
            return Err(WindowError::Missing("a data device"));
        };
        let handle = self.queue.handle();
        let source = manager.create_data_source(&handle, ());
        transfer.offer(&source);
        let offered = actions
            .iter()
            .fold(DndAction::empty(), |all, action| all | wire(*action));
        source.set_actions(offered);
        let icon = match (icon, &self.state.compositor, &self.state.shm) {
            (Some(icon), Some(compositor), Some(shm)) => {
                let surface = compositor.create_surface(&handle, ());
                let viewport = self
                    .state
                    .scaling
                    .viewporter
                    .as_ref()
                    .map(|viewporter| viewporter.get_viewport(&surface, &handle, ()));
                let scale = match &viewport {
                    Some(viewport) => {
                        viewport.set_destination(
                            i32::try_from(icon.scene.width.max(1)).unwrap_or(i32::MAX),
                            i32::try_from(icon.scene.height.max(1)).unwrap_or(i32::MAX),
                        );
                        self.scale()
                    }
                    None => 1.0,
                };
                if surface.version() >= 5 {
                    surface.offset(
                        -icon.hotspot.x.round() as i32,
                        -icon.hotspot.y.round() as i32,
                    );
                }
                let mut target = ShmTarget::default();
                target.present(
                    &mut self.cpu,
                    Target {
                        shm,
                        handle: &handle,
                        surface: &surface,
                    },
                    &icon.scene,
                    scale,
                    None,
                )?;
                Some(Icon {
                    surface,
                    viewport,
                    target,
                })
            }
            _ => None,
        };
        device.start_drag(
            Some(&source),
            &self.surface,
            icon.as_ref().map(|icon| &icon.surface),
            serial,
        );
        if let Some(previous) = self.state.clipboard.dragged.replace(Outgoing {
            source,
            transfer,
            action: None,
            icon,
            dropped_at: None,
        }) {
            finish_outgoing(previous);
        }
        self.flush()
    }

    pub fn accept_drag(&mut self, action: Option<DragAction>) {
        self.answer_drag(action, false);
    }

    pub fn accept_drag_only(&mut self, action: Option<DragAction>) {
        self.answer_drag(action, true);
    }

    fn answer_drag(&mut self, action: Option<DragAction>, only: bool) {
        let Some(incoming) = &mut self.state.clipboard.incoming else {
            return;
        };
        incoming.accepted = action;
        incoming.only = only;
        respond(incoming);
        if let Err(error) = self.flush() {
            tracing::warn!(%error, "cannot answer the drag");
        }
    }

    pub(crate) fn transfer_fds(&self) -> Vec<BorrowedFd<'_>> {
        self.state
            .clipboard
            .readings
            .iter()
            .map(|reading| reading.pipe.as_fd())
            .collect()
    }

    pub(crate) fn read_transfer(&mut self, index: usize) {
        let Some(reading) = self.state.clipboard.readings.get_mut(index) else {
            return;
        };
        let mut chunk = [0u8; 4096];
        let finished = loop {
            match reading.pipe.read(&mut chunk) {
                Ok(0) => break true,
                Ok(read) => reading.bytes.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break false,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break true,
            }
        };
        if !finished {
            return;
        }
        let reading = self.state.clipboard.readings.remove(index);
        let transfer = Transfer::parse(&reading.mime, &reading.bytes);
        match reading.purpose {
            Purpose::Paste { transfer: whole } => {
                self.state.events.push(pasted(transfer, whole));
            }
            Purpose::Drop {
                offer,
                position,
                action,
            } => {
                self.dropped(&offer, position, transfer, action);
                if let Err(error) = self.flush() {
                    tracing::warn!(%error, "cannot finish the drop");
                }
            }
            Purpose::Hover { offer } => self.announce(&offer, transfer.uris),
        }
    }

    fn dropped(
        &mut self,
        offer: &WlDataOffer,
        position: Point,
        transfer: Transfer,
        action: Option<DragAction>,
    ) {
        if transfer.is_empty() {
            offer.destroy();
            self.state.drag_event(DragEvent::Left);
            return;
        }
        if offer.version() >= 3 {
            offer.finish();
        }
        offer.destroy();
        self.state.drag_event(DragEvent::Dropped {
            position,
            transfer,
            action,
        });
    }

    pub(crate) fn drag_offers_text(&self) -> bool {
        self.state
            .clipboard
            .incoming
            .as_ref()
            .is_some_and(|incoming| {
                offered_types(&incoming.offer)
                    .iter()
                    .any(|mime| TEXT_TYPES.contains(&mime.as_str()))
            })
    }

    pub(crate) fn drag_deadline(&self) -> Option<Instant> {
        let dropped_at = self.state.clipboard.dragged.as_ref()?.dropped_at?;
        Some(dropped_at + FINISH_WAIT)
    }

    pub(crate) fn abandon_unfinished_drag(&mut self, now: Instant) {
        if self.drag_deadline().is_some_and(|deadline| deadline <= now)
            && let Some(outgoing) = self.state.clipboard.dragged.take()
        {
            finish_outgoing(outgoing);
            self.state.drag_event(DragEvent::Finished(None));
            if let Err(error) = self.flush() {
                tracing::warn!(%error, "cannot cancel the expired drag");
            }
        }
    }

    fn announce(&mut self, offer: &WlDataOffer, uris: Vec<String>) {
        let Some(incoming) =
            self.state.clipboard.incoming.as_mut().filter(|incoming| {
                incoming.offer == *offer && incoming.arrival != Arrival::Announced
            })
        else {
            return;
        };
        incoming.arrival = Arrival::Announced;
        let position = incoming.position;
        let types = offered_types(offer);
        self.state.drag_event(DragEvent::Entered {
            position,
            uris,
            text: TEXT_TYPES
                .iter()
                .any(|text| types.iter().any(|mime| mime == text)),
        });
    }

    pub(crate) fn take_arrival(&mut self) {
        let Some(incoming) = self
            .state
            .clipboard
            .incoming
            .as_mut()
            .filter(|incoming| incoming.arrival == Arrival::Unread)
        else {
            return;
        };
        incoming.arrival = Arrival::Reading;
        let offer = incoming.offer.clone();
        let types = offered_types(&offer);
        let own = self
            .state
            .clipboard
            .dragged
            .as_ref()
            .filter(|_| types.contains(&own_type()))
            .map(|outgoing| outgoing.transfer.uris.clone());
        if let Some(uris) = own {
            self.announce(&offer, uris);
            return;
        }
        if !types.iter().any(|mime| mime == URI_LIST) {
            self.announce(&offer, Vec::new());
            return;
        }
        let purpose = Purpose::Hover {
            offer: offer.clone(),
        };
        if self
            .read_offer(&offer, URI_LIST.to_owned(), purpose)
            .is_err()
        {
            self.announce(&offer, Vec::new());
        }
    }

    pub(crate) fn take_drop(&mut self) {
        let Some(incoming) = self.state.clipboard.pending_drop.take() else {
            return;
        };
        let types = offered_types(&incoming.offer);
        let action = offered_action(&incoming.offer).or(incoming.accepted);
        let position = incoming.position;
        let Some(mime) = readable_type(&types, true).filter(|_| incoming.accepted.is_some()) else {
            incoming.offer.destroy();
            self.state.drag_event(DragEvent::Left);
            return;
        };
        let own = self
            .state
            .clipboard
            .dragged
            .as_ref()
            .filter(|_| types.contains(&own_type()))
            .map(|outgoing| outgoing.transfer.clone());
        if let Some(transfer) = own {
            self.dropped(&incoming.offer, position, transfer, action);
            return;
        }
        let offer = incoming.offer.clone();
        if self
            .read_offer(
                &offer,
                mime,
                Purpose::Drop {
                    offer: incoming.offer,
                    position,
                    action,
                },
            )
            .is_err()
        {
            offer.destroy();
            self.state.drag_event(DragEvent::Left);
        }
    }
}

pub(crate) fn pasted(transfer: Transfer, whole: bool) -> WindowEvent {
    match whole {
        true => WindowEvent::PastedTransfer(transfer),
        false => WindowEvent::Pasted(transfer.text),
    }
}

fn respond(incoming: &Incoming) {
    let types = offered_types(&incoming.offer);
    let mime = incoming.accepted.and_then(|_| readable_type(&types, true));
    incoming.offer.accept(incoming.serial, mime);
    if incoming.offer.version() >= 3 {
        match incoming.accepted {
            Some(action) => incoming.offer.set_actions(
                match incoming.only {
                    true => wire(action),
                    false => DndAction::Copy | DndAction::Move,
                },
                wire(action),
            ),
            None => incoming
                .offer
                .set_actions(DndAction::empty(), DndAction::empty()),
        }
    }
}

fn finish_outgoing(outgoing: Outgoing) {
    outgoing.source.destroy();
    if let Some(mut icon) = outgoing.icon {
        icon.target.destroy();
        if let Some(viewport) = icon.viewport {
            viewport.destroy();
        }
        icon.surface.destroy();
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
        match event {
            wl_data_device::Event::Selection { id } => {
                if let Some(previous) = std::mem::replace(&mut state.clipboard.selection, id) {
                    previous.destroy();
                }
            }
            wl_data_device::Event::Enter {
                surface,
                id: Some(offer),
                ..
            } if state.is_foreign(&surface) => offer.destroy(),
            wl_data_device::Event::Enter {
                serial,
                x,
                y,
                id: Some(offer),
                ..
            } => {
                let incoming = Incoming {
                    offer,
                    serial,
                    position: Point::new(x as f32, y as f32),
                    accepted: None,
                    only: false,
                    arrival: Arrival::Unread,
                };
                respond(&incoming);
                if let Some(previous) = state.clipboard.incoming.replace(incoming) {
                    let announced = previous.arrival == Arrival::Announced;
                    previous.offer.destroy();
                    if announced {
                        state.drag_event(DragEvent::Left);
                    }
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                let position = Point::new(x as f32, y as f32);
                let Some(incoming) = &mut state.clipboard.incoming else {
                    return;
                };
                incoming.position = position;
                respond(incoming);
                if incoming.arrival == Arrival::Announced {
                    state.drag_event(DragEvent::Moved(position));
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(incoming) = state.clipboard.incoming.take() {
                    incoming.offer.destroy();
                    if incoming.arrival == Arrival::Announced {
                        state.drag_event(DragEvent::Left);
                    }
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(dropped) = state.clipboard.incoming.take()
                    && let Some(stale) = state.clipboard.pending_drop.replace(dropped)
                {
                    stale.offer.destroy();
                }
            }
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferState::default()),
    ]);
}

impl Dispatch<WlDataOffer, OfferState> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        offer: &OfferState,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Ok(mut types) = offer.types.lock() {
                    types.push(mime_type);
                }
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(action),
            } => {
                if let Ok(mut chosen) = offer.action.lock() {
                    *chosen = from_wire(action);
                }
            }
            _ => {}
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
        let clipboard = &mut state.clipboard;
        let dragged = clipboard
            .dragged
            .as_mut()
            .filter(|outgoing| outgoing.source == *source);
        match (event, dragged) {
            (wl_data_source::Event::Send { mime_type, fd }, Some(outgoing)) => {
                send(fd, outgoing.transfer.bytes(&mime_type));
            }
            (wl_data_source::Event::Send { mime_type, fd }, None) => {
                if let Some((_, transfer)) = clipboard
                    .copied
                    .as_ref()
                    .filter(|(copied, _)| copied == source)
                {
                    send(fd, transfer.bytes(&mime_type));
                }
            }
            (
                wl_data_source::Event::Action {
                    dnd_action: WEnum::Value(action),
                },
                Some(outgoing),
            ) => outgoing.action = from_wire(action),
            (wl_data_source::Event::DndDropPerformed, Some(outgoing)) => {
                outgoing.dropped_at = Some(Instant::now());
            }
            (wl_data_source::Event::DndFinished, Some(_)) => {
                if let Some(outgoing) = clipboard.dragged.take() {
                    let action = outgoing.action;
                    finish_outgoing(outgoing);
                    state.drag_event(DragEvent::Finished(action));
                }
            }
            (wl_data_source::Event::Cancelled, Some(_)) => {
                if let Some(outgoing) = clipboard.dragged.take() {
                    finish_outgoing(outgoing);
                    state.drag_event(DragEvent::Finished(None));
                }
            }
            (wl_data_source::Event::Cancelled, None) => {
                if clipboard
                    .copied
                    .as_ref()
                    .is_some_and(|(copied, _)| copied == source)
                {
                    clipboard.copied = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

pub(crate) fn send(fd: OwnedFd, bytes: Vec<u8>) {
    let spawned = std::thread::Builder::new().spawn(move || {
        if let Ok(flags) = fcntl_getfl(&fd) {
            let _ = fcntl_setfl(&fd, flags - OFlags::NONBLOCK);
        }
        if let Err(error) = File::from(fd).write_all(&bytes) {
            tracing::debug!(%error, "cannot send the clipboard data");
        }
    });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start the clipboard transfer");
    }
}
