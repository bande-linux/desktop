use std::error::Error;
use std::fs::File;
use std::io::{ErrorKind, PipeReader, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bande_clipboard::{
    BUS_NAME, ClipboardHistory, Contents, History, MAX_BYTES, OBJECT_PATH, Shared, is_sensitive,
    lock, now,
};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{DumpableBehavior, set_dumpable_behavior};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{
    Connection, Dispatch, DispatchError, Proxy, QueueHandle, delegate_noop, event_created_child,
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

const SOURCE_SILENCE_LIMIT: Timespec = Timespec {
    tv_sec: 2,
    tv_nsec: 0,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Selection {
    Clipboard,
    Primary,
}

impl Selection {
    const ALL: [Selection; 2] = [Selection::Clipboard, Selection::Primary];

    fn index(self) -> usize {
        self as usize
    }
}

type Mimes = Mutex<Vec<String>>;

struct Saved {
    selection: Selection,
    contents: Contents,
}

#[derive(Default)]
struct Daemon {
    owned: [Option<ExtDataControlSourceV1>; 2],
    pending: [Option<ExtDataControlOfferV1>; 2],
    finished: bool,
}

impl Daemon {
    fn offered(&mut self, selection: Selection, offer: Option<ExtDataControlOfferV1>) {
        let Some(offer) = offer else {
            return;
        };
        if self.owned[selection.index()].is_some() {
            offer.destroy();
            return;
        }
        if let Some(stale) = self.pending[selection.index()].replace(offer) {
            stale.destroy();
        }
    }
}

fn main() -> ExitCode {
    bande_log::init("bande-clipboard");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if compositor_gone(error.as_ref()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn compositor_gone(error: &(dyn Error + 'static)) -> bool {
    let io = match error.downcast_ref::<DispatchError>() {
        Some(DispatchError::Backend(WaylandError::Io(io))) => Some(io),
        _ => match error.downcast_ref::<WaylandError>() {
            Some(WaylandError::Io(io)) => Some(io),
            _ => error.downcast_ref::<std::io::Error>(),
        },
    };
    io.is_some_and(|io| {
        matches!(
            io.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::UnexpectedEof
        )
    })
}

fn serve_history(
    history: &Shared,
) -> Result<(zbus::blocking::Connection, Receiver<Contents>, UnixStream), Box<dyn Error>> {
    let (paste, pasted) = channel();
    let (wake, woken) = UnixStream::pair()?;
    woken.set_nonblocking(true)?;
    let bus = zbus::blocking::connection::Builder::session()?
        .serve_at(
            OBJECT_PATH,
            ClipboardHistory {
                history: Arc::clone(history),
                paste,
                wake,
            },
        )?
        .name(BUS_NAME)?
        .build()?;
    Ok((bus, pasted, woken))
}

fn history() -> Shared {
    let history = bande_xdg::state_home()
        .map(|state| state.join("bande/clipboard"))
        .map(|dir| History::load(dir, now()))
        .unwrap_or_default();
    Arc::new(Mutex::new(history))
}

fn wait(
    queue: &mut wayland_client::EventQueue<Daemon>,
    woken: Option<&UnixStream>,
) -> Result<(), Box<dyn Error>> {
    let Some(guard) = queue.prepare_read() else {
        return Ok(());
    };
    let wayland_ready = {
        let connection_fd = guard.connection_fd();
        let mut fds = vec![PollFd::new(&connection_fd, PollFlags::IN)];
        if let Some(woken) = woken {
            fds.push(PollFd::new(woken, PollFlags::IN));
        }
        match poll(&mut fds, None) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
        !fds[0].revents().is_empty()
    };
    if wayland_ready {
        match guard.read() {
            Ok(_) => {}
            Err(WaylandError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
    }
    if let Some(mut woken) = woken {
        let mut drained = [0u8; 64];
        while matches!(woken.read(&mut drained), Ok(read) if read > 0) {}
    }
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<Daemon>(&connection)?;
    let qh = queue.handle();
    let seat: WlSeat = globals.bind(&qh, 1..=1, ())?;
    let manager: ExtDataControlManagerV1 = globals.bind(&qh, 1..=1, ())?;
    let device = manager.get_data_device(&seat, &qh, ());
    let mut daemon = Daemon::default();
    let history = history();
    let served = serve_history(&history)
        .inspect_err(|error| tracing::warn!("cannot offer the clipboard history: {error}"))
        .ok();
    let (_bus, pasted, woken) = match served {
        Some((bus, pasted, woken)) => (Some(bus), Some(pasted), Some(woken)),
        None => (None, None, None),
    };

    while !daemon.finished {
        queue.dispatch_pending(&mut daemon)?;
        for contents in pasted.iter().flat_map(Receiver::try_iter) {
            take_over(
                &manager,
                &device,
                &qh,
                &mut daemon,
                Selection::Clipboard,
                contents,
            );
        }
        for selection in Selection::ALL {
            let Some(offer) = daemon.pending[selection.index()].take() else {
                continue;
            };
            let (contents, complete) = read_offer(&connection, &offer)?;
            offer.destroy();
            queue.roundtrip(&mut daemon)?;
            if daemon.pending[selection.index()].is_some()
                || contents.is_empty()
                || is_sensitive(&contents)
            {
                continue;
            }
            if selection == Selection::Clipboard {
                lock(&history).record(contents.clone(), now());
            }
            if !complete {
                continue;
            }
            take_over(&manager, &device, &qh, &mut daemon, selection, contents);
        }
        connection.flush()?;
        if !daemon.finished {
            wait(&mut queue, woken.as_ref())?;
        }
    }
    Ok(())
}

fn read_offer(
    connection: &Connection,
    offer: &ExtDataControlOfferV1,
) -> Result<(Contents, bool), Box<dyn Error>> {
    let mimes = offer
        .data::<Mimes>()
        .and_then(|mimes| mimes.lock().ok().map(|mimes| mimes.clone()))
        .unwrap_or_default();
    let mut contents = Vec::with_capacity(mimes.len());
    let mut complete = true;
    let mut room = MAX_BYTES;
    for mime in mimes {
        let (reader, writer) = std::io::pipe()?;
        offer.receive(mime.clone(), writer.as_fd());
        connection.flush()?;
        drop(writer);
        match read_until_closed(reader, room)? {
            Some(bytes) => {
                room -= bytes.len();
                contents.push((mime, bytes));
            }
            None => complete = false,
        }
    }
    Ok((contents, complete))
}

fn read_until_closed(mut reader: PipeReader, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let mut fds = [PollFd::new(&reader, PollFlags::IN)];
        if poll(&mut fds, Some(&SOURCE_SILENCE_LIMIT))? == 0 {
            return Ok(None);
        }
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(Some(bytes)),
            Ok(read) if bytes.len() + read > limit => return Ok(None),
            Ok(read) => bytes.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn take_over(
    manager: &ExtDataControlManagerV1,
    device: &ExtDataControlDeviceV1,
    qh: &QueueHandle<Daemon>,
    daemon: &mut Daemon,
    selection: Selection,
    contents: Contents,
) {
    let mimes: Vec<String> = contents.iter().map(|(mime, _)| mime.clone()).collect();
    let source = manager.create_data_source(
        qh,
        Arc::new(Saved {
            selection,
            contents,
        }),
    );
    for mime in mimes {
        source.offer(mime);
    }
    match selection {
        Selection::Clipboard => device.set_selection(Some(&source)),
        Selection::Primary => device.set_primary_selection(Some(&source)),
    }
    if let Some(replaced) = daemon.owned[selection.index()].replace(source) {
        replaced.destroy();
    }
}

fn serve(saved: Arc<Saved>, mime: String, fd: OwnedFd) {
    std::thread::spawn(move || {
        if let Some((_, bytes)) = saved.contents.iter().find(|(offered, _)| *offered == mime) {
            let _ = File::from(fd).write_all(bytes);
        }
    });
}

impl Dispatch<ExtDataControlDeviceV1, ()> for Daemon {
    fn event(
        daemon: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::Selection { id } => {
                daemon.offered(Selection::Clipboard, id)
            }
            ext_data_control_device_v1::Event::PrimarySelection { id } => {
                daemon.offered(Selection::Primary, id)
            }
            ext_data_control_device_v1::Event::Finished => daemon.finished = true,
            _ => {}
        }
    }

    event_created_child!(Daemon, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, Mimes::default()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, Mimes> for Daemon {
    fn event(
        _: &mut Self,
        _: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        mimes: &Mimes,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Ok(mut mimes) = mimes.lock()
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, Arc<Saved>> for Daemon {
    fn event(
        daemon: &mut Self,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        saved: &Arc<Saved>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                serve(Arc::clone(saved), mime_type, fd)
            }
            ext_data_control_source_v1::Event::Cancelled => {
                let owned = &mut daemon.owned[saved.selection.index()];
                if owned.as_ref() == Some(source) {
                    *owned = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Daemon {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(Daemon: ignore WlSeat);
delegate_noop!(Daemon: ExtDataControlManagerV1);
