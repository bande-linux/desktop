use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::AsFd;

use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_shm::Format;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
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

use unbound_protocols::client::unbound_capture_v1::{self, UnboundCaptureV1};

use crate::{Globals, Headless, TestClient};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureOutcome {
    Ready(Vec<u8>),
    Failed(u32),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureConstraints {
    pub size: Option<(u32, u32)>,
    pub shm_formats: Vec<Format>,
    pub done: bool,
    pub stopped: bool,
}

#[derive(Default)]
pub(crate) struct Capture {
    session: Option<ExtImageCopyCaptureSessionV1>,
    constraints: CaptureConstraints,
    memory: Option<File>,
    outcome: Option<CaptureOutcome>,
    shell: Option<UnboundCaptureV1>,
    windows: Vec<(String, (i32, i32, i32, i32))>,
    windows_done: bool,
    option_requests: usize,
}

impl TestClient {
    pub fn can_capture_screen(&self) -> bool {
        self.advertises("ext_output_image_capture_source_manager_v1")
            && self.advertises("ext_image_copy_capture_manager_v1")
    }

    pub fn start_screen_capture(
        &mut self,
        server: &mut Headless,
        paint_cursors: bool,
    ) -> CaptureConstraints {
        let qh = self.queue.handle();
        let output: WlOutput = self.bind("wl_output", 1);
        let sources: ExtOutputImageCaptureSourceManagerV1 =
            self.bind("ext_output_image_capture_source_manager_v1", 1);
        let source = sources.create_source(&output, &qh, ());
        self.start_capture_session(server, &source, paint_cursors)
    }

    pub fn start_window_capture(
        &mut self,
        server: &mut Headless,
        app_id: &str,
        paint_cursors: bool,
    ) -> CaptureConstraints {
        let qh = self.queue.handle();
        let handle = self
            .toplevel_handle(app_id)
            .expect("join_shell first and show a window with this app id");
        let sources: ExtForeignToplevelImageCaptureSourceManagerV1 =
            self.bind("ext_foreign_toplevel_image_capture_source_manager_v1", 1);
        let source = sources.create_source(&handle, &qh, ());
        self.start_capture_session(server, &source, paint_cursors)
    }

    fn start_capture_session(
        &mut self,
        server: &mut Headless,
        source: &ExtImageCaptureSourceV1,
        paint_cursors: bool,
    ) -> CaptureConstraints {
        let qh = self.queue.handle();
        let manager: ExtImageCopyCaptureManagerV1 =
            self.bind("ext_image_copy_capture_manager_v1", 1);
        let options = if paint_cursors {
            Options::PaintCursors
        } else {
            Options::empty()
        };
        self.globals.capture = Capture {
            session: Some(manager.create_session(source, options, &qh, ())),
            ..Capture::default()
        };
        self.exchange(server);
        self.globals.capture.constraints.clone()
    }

    pub fn capture_frame(&mut self, server: &mut Headless, (width, height): (i32, i32)) {
        let qh = self.queue.handle();
        let stride = width * 4;
        let file = tempfile::tempfile().expect("capture file");
        file.set_len(u64::try_from(stride * height).expect("capture size"))
            .expect("capture length");
        let shm = self.globals.shm.as_ref().expect("wl_shm");
        let pool = shm.create_pool(file.as_fd(), stride * height, &qh, ());
        let buffer = pool.create_buffer(0, width, height, stride, Format::Argb8888, &qh, ());
        pool.destroy();
        let session = self
            .globals
            .capture
            .session
            .as_ref()
            .expect("start_screen_capture first");
        let frame = session.create_frame(&qh, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, width, height);
        frame.capture();
        self.globals.capture.memory = Some(file);
        self.globals.capture.outcome = None;
        self.exchange(server);
    }

    pub fn capture_outcome(&self) -> Option<&CaptureOutcome> {
        self.globals.capture.outcome.as_ref()
    }

    pub fn capture_constraints(&self) -> &CaptureConstraints {
        &self.globals.capture.constraints
    }

    pub fn join_capture(&mut self, server: &mut Headless) {
        let capture: UnboundCaptureV1 = self.bind("unbound_capture_v1", 1);
        self.globals.capture.shell = Some(capture);
        self.exchange(server);
    }

    pub fn list_windows(&mut self, server: &mut Headless) -> Vec<(String, (i32, i32, i32, i32))> {
        let capture = &mut self.globals.capture;
        capture.windows.clear();
        capture.windows_done = false;
        capture
            .shell
            .as_ref()
            .expect("join_capture first")
            .list_windows();
        self.exchange(server);
        assert!(
            self.globals.capture.windows_done,
            "windows_done follows the list"
        );
        self.globals.capture.windows.clone()
    }

    pub fn capture_option_requests(&self) -> usize {
        self.globals.capture.option_requests
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let constraints = &mut globals.capture.constraints;
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                constraints.size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => constraints.shm_formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => constraints.done = true,
            ext_image_copy_capture_session_v1::Event::Stopped => constraints.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let capture = &mut globals.capture;
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => {
                let mut pixels = Vec::new();
                if let Some(file) = capture.memory.as_mut() {
                    file.seek(SeekFrom::Start(0)).expect("capture seek");
                    file.read_to_end(&mut pixels).expect("capture read");
                }
                capture.outcome = Some(CaptureOutcome::Ready(pixels));
                frame.destroy();
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                capture.outcome = Some(CaptureOutcome::Failed(reason.into()));
                frame.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<UnboundCaptureV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &UnboundCaptureV1,
        event: unbound_capture_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let capture = &mut globals.capture;
        match event {
            unbound_capture_v1::Event::Window {
                identifier,
                x,
                y,
                width,
                height,
            } => capture.windows.push((identifier, (x, y, width, height))),
            unbound_capture_v1::Event::WindowsDone => capture.windows_done = true,
            unbound_capture_v1::Event::ShowOptions => capture.option_requests += 1,
            _ => {}
        }
    }
}

delegate_noop!(Globals: ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Globals: ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(Globals: ExtImageCaptureSourceV1);
delegate_noop!(Globals: ExtImageCopyCaptureManagerV1);
