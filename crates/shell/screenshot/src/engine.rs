use std::collections::VecDeque;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{MemfdFlags, memfd_create};
use unbound_inbox::{Inbox, Outbox};
use unbound_protocols::client::unbound_capture_v1::{self, UnboundCaptureV1};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1, FailureReason,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    ExtImageCopyCaptureManagerV1, Options,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};

const FORMAT: Format = Format::Argb8888;
const BYTES_PER_PIXEL: u32 = 4;
const MOST_FRAMES_PER_SECOND: u32 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Screen,
    Display(String),
    Window(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub name: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub identifier: String,
    pub title: String,
    pub app_id: String,
    pub frame: Option<Frame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl Shot {
    pub fn rgba(&self) -> Vec<u8> {
        self.bgra
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|&[blue, green, red, alpha]| {
                let straight = |channel: u8| match alpha {
                    0 => 0,
                    _ => ((u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha))
                        .min(255) as u8,
                };
                [straight(red), straight(green), straight(blue), alpha]
            })
            .collect()
    }
}

pub type FrameSink = Box<dyn FnMut(u32, u32, &[u8]) + Send>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineEvent {
    ShowOptions,
    StreamEnded(u64),
}

enum Command {
    Windows(Sender<Vec<WindowInfo>>),
    Displays(Sender<Vec<Display>>),
    Shoot {
        source: Source,
        cursor: bool,
        reply: Sender<Option<Shot>>,
    },
    Stream {
        id: u64,
        source: Source,
        cursor: bool,
        sink: FrameSink,
    },
    Stop(u64),
}

#[derive(Clone)]
pub struct Engine {
    commands: Outbox<Command>,
}

impl Engine {
    pub fn start(connection: Connection, events: Outbox<EngineEvent>) -> io::Result<Self> {
        let inbox = Inbox::new()?;
        let commands = inbox.outbox();
        let (globals, queue) = registry_queue_init::<Capture>(&connection)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let handle = queue.handle();
        let mut capture = Capture {
            shm: globals.bind(&handle, 1..=1, ()).ok(),
            displays: Vec::new(),
            output_sources: globals.bind(&handle, 1..=1, ()).ok(),
            toplevel_sources: globals.bind(&handle, 1..=1, ()).ok(),
            copy: globals.bind(&handle, 1..=1, ()).ok(),
            shell: globals.bind(&handle, 1..=1, ()).ok(),
            toplevels: Vec::new(),
            listed: Vec::new(),
            listings: VecDeque::new(),
            sessions: Vec::new(),
            next_session: u64::MAX / 2,
            events,
        };
        if capture.copy.is_none() {
            return Err(io::Error::other(
                "the compositor does not offer ext_image_copy_capture_manager_v1",
            ));
        }
        let _: Option<ExtForeignToplevelListV1> = globals.bind(&handle, 1..=1, ()).ok();
        for global in globals.contents().clone_list() {
            if global.interface == "wl_output" {
                let output: WlOutput = globals.registry().bind(
                    global.name,
                    global.version.min(4),
                    &handle,
                    global.name,
                );
                capture.displays.push(Output {
                    output,
                    global: global.name,
                    display: Display {
                        name: String::new(),
                        description: String::new(),
                        x: 0,
                        y: 0,
                    },
                });
            }
        }
        thread::Builder::new()
            .name("capture".into())
            .spawn(move || {
                if let Err(error) = run(queue, &mut capture, inbox) {
                    tracing::warn!("capture stopped: {error}");
                }
            })?;
        Ok(Self { commands })
    }

    pub fn displays(&self) -> Vec<Display> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(Command::Displays(reply));
        answer.recv().unwrap_or_default()
    }

    pub fn windows(&self) -> Vec<WindowInfo> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(Command::Windows(reply));
        answer.recv().unwrap_or_default()
    }

    pub fn shoot(&self, source: Source, cursor: bool) -> Option<Shot> {
        let (reply, answer) = mpsc::channel();
        self.commands.send(Command::Shoot {
            source,
            cursor,
            reply,
        });
        answer.recv().ok().flatten()
    }

    pub fn stream(&self, id: u64, source: Source, cursor: bool, sink: FrameSink) {
        self.commands.send(Command::Stream {
            id,
            source,
            cursor,
            sink,
        });
    }

    pub fn stop(&self, id: u64) {
        self.commands.send(Command::Stop(id));
    }
}

struct Output {
    output: WlOutput,
    global: u32,
    display: Display,
}

struct Toplevel {
    handle: ExtForeignToplevelHandleV1,
    identifier: String,
    title: String,
    app_id: String,
}

enum Sink {
    Once(Sender<Option<Shot>>),
    Stream(u64, FrameSink),
}

struct Buffer {
    buffer: WlBuffer,
    memory: File,
    width: u32,
    height: u32,
}

struct Session {
    id: u64,
    source: ExtImageCaptureSourceV1,
    session: ExtImageCopyCaptureSessionV1,
    size: Option<(u32, u32)>,
    formats: Vec<Format>,
    ready: bool,
    buffer: Option<Buffer>,
    frame: Option<ExtImageCopyCaptureFrameV1>,
    pixels: Vec<u8>,
    next_frame: Option<Instant>,
    sink: Sink,
    ended: bool,
}

struct Capture {
    shm: Option<WlShm>,
    displays: Vec<Output>,
    output_sources: Option<ExtOutputImageCaptureSourceManagerV1>,
    toplevel_sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    copy: Option<ExtImageCopyCaptureManagerV1>,
    shell: Option<UnboundCaptureV1>,
    toplevels: Vec<Toplevel>,
    listed: Vec<(String, Frame)>,
    listings: VecDeque<Sender<Vec<WindowInfo>>>,
    sessions: Vec<Session>,
    next_session: u64,
    events: Outbox<EngineEvent>,
}

fn run(
    mut queue: EventQueue<Capture>,
    capture: &mut Capture,
    mut inbox: Inbox<Command>,
) -> io::Result<()> {
    let wayland = |error: WaylandError| io::Error::other(error.to_string());
    loop {
        queue
            .dispatch_pending(capture)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let now = Instant::now();
        for command in inbox.drain() {
            capture.command(command, &queue.handle());
        }
        capture.request_due_frames(now, &queue.handle());
        capture.sessions.retain(|session| !session.ended);
        queue.flush().map_err(wayland)?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let deadline = capture
            .sessions
            .iter()
            .filter_map(|session| session.next_frame)
            .min();
        let wayland_ready = {
            let connection = guard.connection_fd();
            let commands = inbox.fd();
            let mut fds = [
                PollFd::new(&connection, PollFlags::IN),
                PollFd::new(&commands, PollFlags::IN),
            ];
            let timeout = deadline.map(|deadline| {
                let wait = deadline.saturating_duration_since(Instant::now());
                Timespec {
                    tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
                    tv_nsec: i64::from(wait.subsec_nanos()),
                }
            });
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(io::Error::from(error)),
            }
            !fds[0].revents().is_empty()
        };
        if wayland_ready {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(wayland(error)),
            }
        } else {
            drop(guard);
        }
    }
}

impl Capture {
    fn command(&mut self, command: Command, handle: &QueueHandle<Self>) {
        match command {
            Command::Windows(reply) => match &self.shell {
                Some(shell) => {
                    shell.list_windows();
                    self.listings.push_back(reply);
                }
                None => {
                    let _ = reply.send(self.window_infos(&[]));
                }
            },
            Command::Displays(reply) => {
                let _ = reply.send(
                    self.displays
                        .iter()
                        .map(|output| output.display.clone())
                        .collect(),
                );
            }
            Command::Shoot {
                source,
                cursor,
                reply,
            } => {
                if !self.open(&source, cursor, Sink::Once(reply.clone()), handle) {
                    let _ = reply.send(None);
                }
            }
            Command::Stream {
                id,
                source,
                cursor,
                sink,
            } => {
                if !self.open(&source, cursor, Sink::Stream(id, sink), handle) {
                    self.events.send(EngineEvent::StreamEnded(id));
                }
            }
            Command::Stop(id) => {
                if let Some(session) = self
                    .sessions
                    .iter_mut()
                    .find(|session| matches!(session.sink, Sink::Stream(stream, _) if stream == id))
                {
                    session.close();
                }
            }
        }
    }

    fn window_infos(&self, listed: &[(String, Frame)]) -> Vec<WindowInfo> {
        let info = |toplevel: &Toplevel, frame| WindowInfo {
            identifier: toplevel.identifier.clone(),
            title: toplevel.title.clone(),
            app_id: toplevel.app_id.clone(),
            frame,
        };
        let shown = listed.iter().filter_map(|(identifier, frame)| {
            self.toplevels
                .iter()
                .find(|toplevel| &toplevel.identifier == identifier)
                .map(|toplevel| info(toplevel, Some(*frame)))
        });
        let hidden = self
            .toplevels
            .iter()
            .filter(|toplevel| {
                !listed
                    .iter()
                    .any(|(identifier, _)| identifier == &toplevel.identifier)
            })
            .map(|toplevel| info(toplevel, None));
        shown.chain(hidden).collect()
    }

    fn open(
        &mut self,
        source: &Source,
        cursor: bool,
        sink: Sink,
        handle: &QueueHandle<Self>,
    ) -> bool {
        let (Some(copy), Some(_)) = (&self.copy, &self.shm) else {
            return false;
        };
        let captured = match source {
            Source::Screen | Source::Display(_) => {
                let output = match source {
                    Source::Display(name) => self
                        .displays
                        .iter()
                        .find(|output| &output.display.name == name),
                    _ => self.displays.first(),
                };
                match (&self.output_sources, output) {
                    (Some(sources), Some(output)) => {
                        sources.create_source(&output.output, handle, ())
                    }
                    _ => return false,
                }
            }
            Source::Window(identifier) => {
                let toplevel = self
                    .toplevels
                    .iter()
                    .find(|toplevel| &toplevel.identifier == identifier);
                match (&self.toplevel_sources, toplevel) {
                    (Some(sources), Some(toplevel)) => {
                        sources.create_source(&toplevel.handle, handle, ())
                    }
                    _ => return false,
                }
            }
        };
        let options = match cursor {
            true => Options::PaintCursors,
            false => Options::empty(),
        };
        self.next_session += 1;
        let id = self.next_session;
        let session = copy.create_session(&captured, options, handle, id);
        self.sessions.push(Session {
            id,
            source: captured,
            session,
            size: None,
            formats: Vec::new(),
            ready: false,
            buffer: None,
            frame: None,
            pixels: Vec::new(),
            next_frame: None,
            sink,
            ended: false,
        });
        true
    }

    fn request_due_frames(&mut self, now: Instant, handle: &QueueHandle<Self>) {
        let Some(shm) = self.shm.clone() else {
            return;
        };
        for session in &mut self.sessions {
            if session.ended || session.frame.is_some() || !session.ready {
                continue;
            }
            if session.next_frame.is_some_and(|due| due > now) {
                continue;
            }
            session.next_frame = None;
            if let Err(error) = session.request_frame(&shm, handle) {
                tracing::warn!("cannot capture a frame: {error}");
                session.end(&self.events);
            }
        }
    }

    fn session(&mut self, id: u64) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }
}

impl Session {
    fn request_frame(&mut self, shm: &WlShm, handle: &QueueHandle<Capture>) -> io::Result<()> {
        let (width, height) = self
            .size
            .filter(|_| self.formats.contains(&FORMAT))
            .ok_or_else(|| io::Error::other("the compositor offers no usable buffer"))?;
        if self
            .buffer
            .as_ref()
            .is_none_or(|buffer| (buffer.width, buffer.height) != (width, height))
        {
            if let Some(old) = self.buffer.take() {
                old.buffer.destroy();
            }
            self.buffer = Some(allocate(shm, width, height, handle)?);
        }
        let Some(buffer) = &self.buffer else {
            return Ok(());
        };
        let frame = self.session.create_frame(handle, self.id);
        frame.attach_buffer(&buffer.buffer);
        frame.damage_buffer(0, 0, side(width)?, side(height)?);
        frame.capture();
        self.frame = Some(frame);
        Ok(())
    }

    fn deliver(&mut self, events: &Outbox<EngineEvent>) {
        let Some(buffer) = &self.buffer else {
            return;
        };
        let length = (buffer.width * buffer.height * BYTES_PER_PIXEL) as usize;
        self.pixels.resize(length, 0);
        if buffer.memory.read_exact_at(&mut self.pixels, 0).is_err() {
            self.end(events);
            return;
        }
        let (width, height) = (buffer.width, buffer.height);
        match &mut self.sink {
            Sink::Once(reply) => {
                let _ = reply.send(Some(Shot {
                    width,
                    height,
                    bgra: std::mem::take(&mut self.pixels),
                }));
                self.close();
            }
            Sink::Stream(_, sink) => {
                sink(width, height, &self.pixels);
                self.next_frame =
                    Some(Instant::now() + Duration::from_secs(1) / MOST_FRAMES_PER_SECOND);
            }
        }
    }

    fn end(&mut self, events: &Outbox<EngineEvent>) {
        match &self.sink {
            Sink::Once(reply) => {
                let _ = reply.send(None);
            }
            Sink::Stream(id, _) => {
                events.send(EngineEvent::StreamEnded(*id));
            }
        }
        self.close();
    }

    fn close(&mut self) {
        if let Some(frame) = self.frame.take() {
            frame.destroy();
        }
        if let Some(buffer) = self.buffer.take() {
            buffer.buffer.destroy();
        }
        self.session.destroy();
        self.source.destroy();
        self.ended = true;
    }
}

fn side(value: u32) -> io::Result<i32> {
    i32::try_from(value).map_err(|_| io::Error::other("the capture is too large"))
}

fn allocate(
    shm: &WlShm,
    width: u32,
    height: u32,
    handle: &QueueHandle<Capture>,
) -> io::Result<Buffer> {
    let stride = width * BYTES_PER_PIXEL;
    let length = u64::from(stride) * u64::from(height);
    let memory = File::from(memfd_create("unbound-capture", MemfdFlags::CLOEXEC)?);
    memory.set_len(length)?;
    let pool: WlShmPool = shm.create_pool(
        memory.as_fd(),
        i32::try_from(length).map_err(|_| io::Error::other("the capture is too large"))?,
        handle,
        (),
    );
    let buffer = pool.create_buffer(
        0,
        side(width)?,
        side(height)?,
        side(stride)?,
        FORMAT,
        handle,
        (),
    );
    pool.destroy();
    Ok(Buffer {
        buffer,
        memory,
        width,
        height,
    })
}

impl Dispatch<ExtImageCopyCaptureSessionV1, u64> for Capture {
    fn event(
        capture: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let events = capture.events.clone();
        let Some(session) = capture.session(*id) else {
            return;
        };
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                session.size = Some((width, height));
                session.formats.clear();
                session.ready = false;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => session.formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => session.ready = true,
            ext_image_copy_capture_session_v1::Event::Stopped => session.end(&events),
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, u64> for Capture {
    fn event(
        capture: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let events = capture.events.clone();
        let Some(session) = capture.session(*id) else {
            return;
        };
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => {
                if let Some(frame) = session.frame.take() {
                    frame.destroy();
                }
                session.deliver(&events);
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                if let Some(frame) = session.frame.take() {
                    frame.destroy();
                }
                if reason != WEnum::Value(FailureReason::BufferConstraints) {
                    session.end(&events);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<UnboundCaptureV1, ()> for Capture {
    fn event(
        capture: &mut Self,
        _: &UnboundCaptureV1,
        event: unbound_capture_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            unbound_capture_v1::Event::Window {
                identifier,
                x,
                y,
                width,
                height,
            } => capture.listed.push((
                identifier,
                Frame {
                    x,
                    y,
                    width,
                    height,
                },
            )),
            unbound_capture_v1::Event::WindowsDone => {
                let listed = std::mem::take(&mut capture.listed);
                if let Some(reply) = capture.listings.pop_front() {
                    let _ = reply.send(capture.window_infos(&listed));
                }
            }
            unbound_capture_v1::Event::ShowOptions => {
                capture.events.send(EngineEvent::ShowOptions);
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Capture {
    fn event(
        capture: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            capture.toplevels.push(Toplevel {
                handle: toplevel,
                identifier: String::new(),
                title: String::new(),
                app_id: String::new(),
            });
        }
    }

    event_created_child!(Capture, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Capture {
    fn event(
        capture: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_handle_v1::Event::Closed = event {
            capture
                .toplevels
                .retain(|toplevel| &toplevel.handle != handle);
            handle.destroy();
            return;
        }
        let Some(toplevel) = capture
            .toplevels
            .iter_mut()
            .find(|toplevel| &toplevel.handle == handle)
        else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                toplevel.identifier = identifier;
            }
            ext_foreign_toplevel_handle_v1::Event::Title { title } => toplevel.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => toplevel.app_id = app_id,
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Capture {
    fn event(
        capture: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        handle: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_output" => {
                let output = registry.bind(name, version.min(4), handle, name);
                capture.displays.push(Output {
                    output,
                    global: name,
                    display: Display {
                        name: String::new(),
                        description: String::new(),
                        x: 0,
                        y: 0,
                    },
                });
            }
            wl_registry::Event::GlobalRemove { name } => {
                capture.displays.retain(|output| output.global != name);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for Capture {
    fn event(
        capture: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        global: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(output) = capture
            .displays
            .iter_mut()
            .find(|output| output.global == *global)
        else {
            return;
        };
        match event {
            wl_output::Event::Geometry { x, y, .. } => {
                output.display.x = x;
                output.display.y = y;
            }
            wl_output::Event::Name { name } => output.display.name = name,
            wl_output::Event::Description { description } => {
                output.display.description = description;
            }
            _ => {}
        }
    }
}

delegate_noop!(Capture: ignore WlShm);
delegate_noop!(Capture: WlShmPool);
delegate_noop!(Capture: ignore WlBuffer);
delegate_noop!(Capture: ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Capture: ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(Capture: ExtImageCaptureSourceV1);
delegate_noop!(Capture: ExtImageCopyCaptureManagerV1);

#[cfg(test)]
mod tests {
    use super::Shot;

    #[test]
    fn premultiplied_bgra_becomes_straight_rgba() {
        let shot = Shot {
            width: 2,
            height: 1,
            bgra: vec![0x30, 0x20, 0x10, 0xff, 0x40, 0x00, 0x00, 0x80],
        };

        assert_eq!(
            shot.rgba(),
            vec![0x10, 0x20, 0x30, 0xff, 0x00, 0x00, 0x80, 0x80]
        );
    }
}
