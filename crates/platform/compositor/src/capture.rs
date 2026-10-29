use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::Offscreen;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::{
    ext_image_capture_source_v1::{self, ExtImageCaptureSourceV1},
    ext_output_image_capture_source_manager_v1::{self, ExtOutputImageCaptureSourceManagerV1},
};
use smithay::reexports::wayland_protocols::ext::image_copy_capture::v1::server::{
    ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
    ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_output::Transform as WlTransform;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::utils::{Clock, Monotonic, Size, Transform};
use smithay::wayland::shm::with_buffer_contents_mut;
use unbound_render::{RenderError, VulkanRenderer};

use crate::render::{background, rendering_error};
use crate::shell::is_shell;
use crate::state::{Headless, State};

const SHM_FORMATS: [Format; 2] = [Format::Argb8888, Format::Xrgb8888];
const BYTES_PER_PIXEL: usize = 4;

pub(crate) struct PendingCapture {
    frame: ExtImageCopyCaptureFrameV1,
    buffer: WlBuffer,
    paint_cursors: bool,
}

pub(crate) struct SourceData {
    shows_output: bool,
}

pub(crate) struct SessionData {
    live: bool,
    paint_cursors: bool,
    frames: AtomicUsize,
}

pub(crate) struct CursorSessionData {
    session_taken: AtomicBool,
}

pub(crate) struct FrameData {
    session: ExtImageCopyCaptureSessionV1,
    buffer: Mutex<Option<WlBuffer>>,
    captured: AtomicBool,
}

impl State {
    pub(crate) fn create_capture_globals(display: &DisplayHandle) {
        display.create_global::<State, ExtOutputImageCaptureSourceManagerV1, ()>(1, ());
        display.create_global::<State, ExtImageCopyCaptureManagerV1, ()>(1, ());
    }
}

impl Headless {
    pub(crate) fn copy_captures(&mut self, renderer: &mut VulkanRenderer) {
        let pending = std::mem::take(&mut self.state.pending_captures);
        if pending.is_empty() {
            return;
        }
        let mut without_cursor = None;
        let mut with_cursor = None;
        for capture in pending {
            let image = if capture.paint_cursors {
                &mut with_cursor
            } else {
                &mut without_cursor
            };
            if image.is_none() {
                *image = Some(self.capture_image(renderer, capture.paint_cursors));
            }
            let outcome = match image {
                Some(Ok(pixels)) => write_into(&capture.buffer, self.output_size(), pixels),
                _ => Err(FailureReason::Unknown),
            };
            match outcome {
                Ok(()) => {
                    let size = self.output_size();
                    let now = Duration::from(Clock::<Monotonic>::new().now());
                    let seconds = now.as_secs();
                    capture.frame.transform(WlTransform::Normal);
                    capture.frame.damage(0, 0, size.w, size.h);
                    capture.frame.presentation_time(
                        (seconds >> 32) as u32,
                        seconds as u32,
                        now.subsec_nanos(),
                    );
                    capture.frame.ready();
                }
                Err(reason) => capture.frame.failed(reason),
            }
        }
    }

    fn capture_image(
        &self,
        renderer: &mut VulkanRenderer,
        paint_cursors: bool,
    ) -> Result<Vec<u8>, RenderError> {
        let size = self.output_size();
        let elements = self.scene_elements(renderer, paint_cursors);
        let mut target = renderer.create_buffer(Fourcc::Argb8888, Size::from((size.w, size.h)))?;
        let mut damage = OutputDamageTracker::new(size, self.output_scale(), Transform::Normal);
        {
            let mut framebuffer = smithay::backend::renderer::Bind::bind(renderer, &mut target)?;
            damage
                .render_output(renderer, &mut framebuffer, 0, &elements, background())
                .map_err(rendering_error)?;
        }
        renderer.read_target(&target)
    }
}

fn write_into(
    buffer: &WlBuffer,
    size: Size<i32, smithay::utils::Physical>,
    pixels: &[u8],
) -> Result<(), FailureReason> {
    with_buffer_contents_mut(buffer, |memory, length, data| {
        if data.width != size.w || data.height != size.h || !SHM_FORMATS.contains(&data.format) {
            return Err(FailureReason::BufferConstraints);
        }
        let row = data.width.unsigned_abs() as usize * BYTES_PER_PIXEL;
        let stride = data.stride.unsigned_abs() as usize;
        let offset = data.offset.unsigned_abs() as usize;
        let rows = data.height.unsigned_abs() as usize;
        if data.stride < 0
            || stride < row
            || offset + stride * rows.saturating_sub(1) + row > length
        {
            return Err(FailureReason::BufferConstraints);
        }
        for (index, source) in pixels.chunks_exact(row).take(rows).enumerate() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    source.as_ptr(),
                    memory.add(offset + index * stride),
                    row,
                );
            }
        }
        Ok(())
    })
    .map_err(|_| FailureReason::BufferConstraints)?
}

impl GlobalDispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ExtOutputImageCaptureSourceManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        is_shell(&client)
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ExtOutputImageCaptureSourceManagerV1,
        request: ext_output_image_capture_source_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_output_image_capture_source_manager_v1::Request::CreateSource {
            source,
            output,
        } = request
        {
            let shows_output = state.output.owns(&output);
            data_init.init(source, SourceData { shows_output });
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, SourceData> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ExtImageCaptureSourceV1,
        _request: ext_image_capture_source_v1::Request,
        _data: &SourceData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ExtImageCopyCaptureManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        is_shell(&client)
    }
}

fn start_session(
    state: &State,
    session: New<ExtImageCopyCaptureSessionV1>,
    live: bool,
    paint_cursors: bool,
    data_init: &mut DataInit<'_, State>,
) {
    let session = data_init.init(
        session,
        SessionData {
            live,
            paint_cursors,
            frames: AtomicUsize::new(0),
        },
    );
    if !live {
        session.stopped();
        return;
    }
    let size = state.output_size;
    session.buffer_size(size.w.unsigned_abs(), size.h.unsigned_abs());
    for format in SHM_FORMATS {
        session.shm_format(format);
    }
    session.done();
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession {
                session,
                source,
                options,
            } => {
                let WEnum::Value(options) = options else {
                    resource.post_error(
                        ext_image_copy_capture_manager_v1::Error::InvalidOption,
                        "unknown capture option",
                    );
                    return;
                };
                let live = source
                    .data::<SourceData>()
                    .is_some_and(|source| source.shows_output);
                let paint_cursors = options.contains(Options::PaintCursors);
                start_session(state, session, live, paint_cursors, data_init);
            }
            ext_image_copy_capture_manager_v1::Request::CreatePointerCursorSession {
                session,
                ..
            } => {
                data_init.init(
                    session,
                    CursorSessionData {
                        session_taken: AtomicBool::new(false),
                    },
                );
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, CursorSessionData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        data: &CursorSessionData,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } =
            request
        {
            if data.session_taken.swap(true, Ordering::Relaxed) {
                resource.post_error(
                    ext_image_copy_capture_cursor_session_v1::Error::DuplicateSession,
                    "the cursor session already has a capture session",
                );
                return;
            }
            start_session(state, session, false, false, data_init);
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, SessionData> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        resource: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        data: &SessionData,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            if data.frames.fetch_add(1, Ordering::Relaxed) > 0 {
                resource.post_error(
                    ext_image_copy_capture_session_v1::Error::DuplicateFrame,
                    "the session already has a frame",
                );
                return;
            }
            data_init.init(
                frame,
                FrameData {
                    session: resource.clone(),
                    buffer: Mutex::new(None),
                    captured: AtomicBool::new(false),
                },
            );
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, FrameData> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        data: &FrameData,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let already_captured = || {
            resource.post_error(
                ext_image_copy_capture_frame_v1::Error::AlreadyCaptured,
                "the frame was already captured",
            );
        };
        match request {
            ext_image_copy_capture_frame_v1::Request::AttachBuffer { buffer } => {
                if data.captured.load(Ordering::Relaxed) {
                    return already_captured();
                }
                if let Ok(mut attached) = data.buffer.lock() {
                    *attached = Some(buffer);
                }
            }
            ext_image_copy_capture_frame_v1::Request::DamageBuffer {
                x,
                y,
                width,
                height,
            } => {
                if data.captured.load(Ordering::Relaxed) {
                    return already_captured();
                }
                if x < 0 || y < 0 || width <= 0 || height <= 0 {
                    resource.post_error(
                        ext_image_copy_capture_frame_v1::Error::InvalidBufferDamage,
                        "buffer damage must be inside the buffer",
                    );
                }
            }
            ext_image_copy_capture_frame_v1::Request::Capture => {
                if data.captured.swap(true, Ordering::Relaxed) {
                    return already_captured();
                }
                let Some(buffer) = data.buffer.lock().ok().and_then(|buffer| buffer.clone()) else {
                    resource.post_error(
                        ext_image_copy_capture_frame_v1::Error::NoBuffer,
                        "capture needs an attached buffer",
                    );
                    return;
                };
                let Some(session) = data.session.data::<SessionData>() else {
                    return resource.failed(FailureReason::Unknown);
                };
                if !session.live {
                    return resource.failed(FailureReason::Stopped);
                }
                state.pending_captures.push(PendingCapture {
                    frame: resource.clone(),
                    buffer,
                    paint_cursors: session.paint_cursors,
                });
                state.needs_redraw = true;
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ExtImageCopyCaptureFrameV1,
        data: &FrameData,
    ) {
        state
            .pending_captures
            .retain(|capture| &capture.frame != resource);
        if let Some(session) = data.session.data::<SessionData>() {
            session.frames.fetch_sub(1, Ordering::Relaxed);
        }
    }
}
