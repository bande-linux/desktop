use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::backend::ObjectId;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
    event_created_child,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    ExtImageCopyCaptureManagerV1, Options,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};

const FORMAT: Format = Format::Xrgb8888;
const BYTES_PER_PIXEL: u32 = 4;

#[derive(Debug)]
pub enum ScreenError {
    Connect(String),
    Untrusted,
    Wayland(String),
    Stopped,
    Failed(String),
    Io(io::Error),
}

impl std::fmt::Display for ScreenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(error) => write!(formatter, "cannot reach the compositor: {error}"),
            Self::Untrusted => write!(
                formatter,
                "the compositor does not offer screen capture to this client; \
                 it must run as a shell client (see ci/review/run.sh)"
            ),
            Self::Wayland(error) => write!(formatter, "Wayland connection failed: {error}"),
            Self::Stopped => write!(formatter, "the compositor stopped the capture"),
            Self::Failed(reason) => write!(formatter, "a frame capture failed: {reason}"),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ScreenError {}

impl From<io::Error> for ScreenError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<wayland_client::DispatchError> for ScreenError {
    fn from(error: wayland_client::DispatchError) -> Self {
        Self::Wayland(error.to_string())
    }
}

#[derive(Default)]
struct State {
    size: Option<(u32, u32)>,
    formats: Vec<Format>,
    constraints_done: bool,
    stopped: bool,
    outcome: Option<Result<(), String>>,
    pending_app_ids: HashMap<ObjectId, String>,
    app_ids: HashMap<ObjectId, String>,
}

pub struct Screen {
    queue: EventQueue<State>,
    state: State,
    session: ExtImageCopyCaptureSessionV1,
    buffer: WlBuffer,
    memory: File,
    width: u32,
    height: u32,
}

impl Screen {
    pub fn connect() -> Result<Self, ScreenError> {
        let connection = Connection::connect_to_env()
            .map_err(|error| ScreenError::Connect(error.to_string()))?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|error| ScreenError::Connect(error.to_string()))?;
        let handle = queue.handle();
        let sources: ExtOutputImageCaptureSourceManagerV1 = globals
            .bind(&handle, 1..=1, ())
            .map_err(|_| ScreenError::Untrusted)?;
        let manager: ExtImageCopyCaptureManagerV1 = globals
            .bind(&handle, 1..=1, ())
            .map_err(|_| ScreenError::Untrusted)?;
        let output: WlOutput = globals
            .bind(&handle, 1..=4, ())
            .map_err(|error| ScreenError::Connect(error.to_string()))?;
        let shm: WlShm = globals
            .bind(&handle, 1..=1, ())
            .map_err(|error| ScreenError::Connect(error.to_string()))?;
        let toplevels: Result<ExtForeignToplevelListV1, _> = globals.bind(&handle, 1..=1, ());
        if toplevels.is_err() {
            tracing::warn!("no toplevel list: wait-for-window will time out");
        }
        let source = sources.create_source(&output, &handle, ());
        let session = manager.create_session(&source, Options::PaintCursors, &handle, ());
        let mut state = State::default();
        while !state.constraints_done && !state.stopped {
            queue.blocking_dispatch(&mut state)?;
        }
        let Some((width, height)) = state.size.filter(|_| !state.stopped) else {
            return Err(ScreenError::Stopped);
        };
        if !state.formats.contains(&FORMAT) {
            return Err(ScreenError::Failed("no XRGB8888 buffers".to_owned()));
        }
        let stride = width * BYTES_PER_PIXEL;
        let length = u64::from(stride) * u64::from(height);
        let memory = File::from(
            memfd_create("bande-review-frame", MemfdFlags::CLOEXEC).map_err(io::Error::from)?,
        );
        memory.set_len(length)?;
        let side = |value: u32| {
            i32::try_from(value).map_err(|_| ScreenError::Failed("screen too large".to_owned()))
        };
        let pool = shm.create_pool(memory.as_fd(), side(stride * height)?, &handle, ());
        let buffer = pool.create_buffer(
            0,
            side(width)?,
            side(height)?,
            side(stride)?,
            FORMAT,
            &handle,
            (),
        );
        pool.destroy();
        Ok(Self {
            queue,
            state,
            session,
            buffer,
            memory,
            width,
            height,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn capture(&mut self, pixels: &mut Vec<u8>) -> Result<(), ScreenError> {
        let handle = self.queue.handle();
        let frame = self.session.create_frame(&handle, ());
        frame.attach_buffer(&self.buffer);
        frame.capture();
        self.state.outcome = None;
        while self.state.outcome.is_none() && !self.state.stopped {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        frame.destroy();
        match self.state.outcome.take() {
            Some(Ok(())) => {}
            Some(Err(reason)) => return Err(ScreenError::Failed(reason)),
            None => return Err(ScreenError::Stopped),
        }
        let length = self.width as usize * self.height as usize * BYTES_PER_PIXEL as usize;
        pixels.resize(length, 0);
        self.memory.read_exact_at(pixels, 0)?;
        Ok(())
    }

    pub fn open_apps(&self) -> Vec<String> {
        self.state.app_ids.values().cloned().collect()
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                state.size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => state.formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => state.constraints_done = true,
            ext_image_copy_capture_session_v1::Event::Stopped => state.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => state.outcome = Some(Ok(())),
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                state.outcome = Some(Err(format!("{reason:?}")));
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtForeignToplevelListV1,
        _: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                state.pending_app_ids.insert(handle.id(), app_id);
            }
            ext_foreign_toplevel_handle_v1::Event::Done => {
                if let Some(app_id) = state.pending_app_ids.remove(&handle.id()) {
                    state.app_ids.insert(handle.id(), app_id);
                }
            }
            ext_foreign_toplevel_handle_v1::Event::Closed => {
                state.pending_app_ids.remove(&handle.id());
                state.app_ids.remove(&handle.id());
                handle.destroy();
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: ignore WlBuffer);
delegate_noop!(State: ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ExtImageCaptureSourceV1);
delegate_noop!(State: ExtImageCopyCaptureManagerV1);
