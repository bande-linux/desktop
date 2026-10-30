use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::damage::OutputDamageTracker;
use smithay::backend::renderer::element::RenderElement;
use smithay::backend::renderer::{Bind, Color32F, Offscreen};
use smithay::reexports::wayland_protocols::ext::image_capture_source::v1::server::{
    ext_foreign_toplevel_image_capture_source_manager_v1::{
        self, ExtForeignToplevelImageCaptureSourceManagerV1,
    },
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
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::utils::{Clock, Monotonic, Physical, Point, Rectangle, Size, Transform};
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle;
use smithay::wayland::shm::with_buffer_contents_mut;
use unbound_render::{RenderError, VulkanRenderer};

use crate::element::ScreenElement;
use crate::motion::Look;
use crate::render::{background, rendering_error};
use crate::scene::{Shading, window_elements};
use crate::shell::is_shell;
use crate::state::{Headless, OutputElement, State};
use crate::window::Window;

const SHM_FORMATS: [Format; 2] = [Format::Argb8888, Format::Xrgb8888];
const BYTES_PER_PIXEL: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Output(String),
    Window(String),
    Nothing,
}

pub(crate) struct PendingCapture {
    frame: ExtImageCopyCaptureFrameV1,
    buffer: WlBuffer,
    session: ExtImageCopyCaptureSessionV1,
}

pub(crate) struct SourceData {
    target: Target,
}

pub(crate) struct SessionData {
    target: Target,
    paint_cursors: bool,
    frames: AtomicUsize,
    announced: Mutex<Option<Size<i32, Physical>>>,
    delivered: AtomicBool,
}

pub(crate) struct CursorSessionData {
    session_taken: AtomicBool,
}

pub(crate) struct FrameData {
    session: ExtImageCopyCaptureSessionV1,
    buffer: Mutex<Option<WlBuffer>>,
    captured: AtomicBool,
}

type Image = Result<Vec<u8>, RenderError>;

impl State {
    pub(crate) fn create_capture_globals(display: &DisplayHandle) {
        display.create_global::<State, ExtOutputImageCaptureSourceManagerV1, ()>(1, ());
        display.create_global::<State, ExtForeignToplevelImageCaptureSourceManagerV1, ()>(1, ());
        display.create_global::<State, ExtImageCopyCaptureManagerV1, ()>(1, ());
    }

    fn captured_window(&self, identifier: &str) -> Option<&Window> {
        self.windows.iter().find(|window| {
            window
                .foreign
                .as_ref()
                .is_some_and(|handle| handle.identifier() == identifier)
        })
    }

    fn window_area(&self, window: &Window) -> Option<(Rectangle<i32, Physical>, f64)> {
        let geometry = window.geometry()?;
        let scale = self.screens[self.screen_of(window)].scale();
        let area = Rectangle::new(geometry.location.to_f64(), geometry.size.to_f64())
            .to_physical(scale)
            .to_i32_round::<i32>();
        (area.size.w > 0 && area.size.h > 0).then_some((area, scale))
    }

    fn capture_size(&self, target: &Target) -> Option<Size<i32, Physical>> {
        match target {
            Target::Output(name) => self
                .screen_named(name)
                .map(|screen| self.screens[screen].mode_size()),
            Target::Window(identifier) => self
                .captured_window(identifier)
                .and_then(|window| self.window_area(window))
                .map(|(area, _)| area.size),
            Target::Nothing => None,
        }
    }
}

impl Headless {
    pub(crate) fn copy_captures(&mut self, renderer: &mut VulkanRenderer, output: &str) {
        let (pending, others): (Vec<PendingCapture>, Vec<PendingCapture>) =
            std::mem::take(&mut self.state.pending_captures)
                .into_iter()
                .partition(|capture| {
                    capture.session.data::<SessionData>().is_none_or(|session| {
                        match &session.target {
                            Target::Output(name) => name == output,
                            Target::Window(_) | Target::Nothing => true,
                        }
                    })
                });
        self.state.pending_captures = others;
        let mut images: Vec<((Target, bool), Image)> = Vec::new();
        let mut withheld = Vec::new();
        for capture in pending {
            let Some(session) = capture.session.data::<SessionData>() else {
                capture.frame.failed(FailureReason::Unknown);
                continue;
            };
            if matches!(session.target, Target::Window(_)) && self.state.is_locked() {
                withheld.push(capture);
                continue;
            }
            let Some(size) = self.state.capture_size(&session.target) else {
                capture.frame.failed(FailureReason::Stopped);
                capture.session.stopped();
                continue;
            };
            if session
                .announced
                .lock()
                .ok()
                .and_then(|announced| *announced)
                != Some(size)
            {
                announce(&capture.session, session, size);
                capture.frame.failed(FailureReason::BufferConstraints);
                continue;
            }
            let key = (session.target.clone(), session.paint_cursors);
            let index = match images.iter().position(|(cached, _)| cached == &key) {
                Some(index) => index,
                None => {
                    let image = self.capture_image(renderer, &key.0, key.1);
                    images.push((key, image));
                    images.len() - 1
                }
            };
            let outcome = match &images[index].1 {
                Ok(pixels) => write_into(&capture.buffer, size, pixels),
                Err(_) => Err(FailureReason::Unknown),
            };
            match outcome {
                Ok(()) => {
                    session.delivered.store(true, Ordering::Relaxed);
                    let transform = match &session.target {
                        Target::Output(name) => self
                            .state
                            .screen_named(name)
                            .map_or(Transform::Normal, |screen| {
                                self.state.screens[screen].settings.transform
                            }),
                        _ => Transform::Normal,
                    };
                    let now = Duration::from(Clock::<Monotonic>::new().now());
                    let seconds = now.as_secs();
                    capture.frame.transform(transform.into());
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
        self.state.pending_captures.extend(withheld);
    }

    fn capture_image(
        &self,
        renderer: &mut VulkanRenderer,
        target: &Target,
        paint_cursors: bool,
    ) -> Image {
        match target {
            Target::Output(name) => {
                let screen = self
                    .state
                    .screen_named(name)
                    .ok_or(RenderError::NothingRendered)?;
                let shown = &self.state.screens[screen];
                let (size, scale, transform) =
                    (shown.mode_size(), shown.scale(), shown.settings.transform);
                let elements = self.scene_elements(renderer, screen, paint_cursors);
                render_image(renderer, size, scale, transform, &elements, background())
            }
            Target::Window(identifier) => {
                let window = self
                    .state
                    .captured_window(identifier)
                    .ok_or(RenderError::NothingRendered)?;
                let (Some((area, scale)), Some(geometry)) =
                    (self.state.window_area(window), window.geometry())
                else {
                    return Err(RenderError::NothingRendered);
                };
                let shading = Shading {
                    active: self.state.key_window().as_ref() == Some(window.surface()),
                    scheme: self.state.decorations.scheme,
                };
                let at_origin = Look::shown(Rectangle::from_size(geometry.size.to_f64()));
                let (shown, _) =
                    window_elements(window, renderer, scale, shading, false, Some(at_origin));
                let cursor = match paint_cursors {
                    true => self.cursor_elements(renderer, scale),
                    false => Vec::new(),
                };
                let to_origin = Point::from((-area.loc.x, -area.loc.y));
                let elements: Vec<ScreenElement> = cursor
                    .into_iter()
                    .map(|element| ScreenElement::new(element, to_origin))
                    .chain(
                        shown
                            .into_iter()
                            .filter(|element| {
                                !matches!(
                                    element,
                                    OutputElement::Shadow(_) | OutputElement::Blur(_)
                                )
                            })
                            .map(|element| ScreenElement::new(element, Point::default())),
                    )
                    .collect();
                render_image(
                    renderer,
                    area.size,
                    scale,
                    Transform::Normal,
                    &elements,
                    Color32F::TRANSPARENT,
                )
            }
            Target::Nothing => Err(RenderError::NothingRendered),
        }
    }
}

fn render_image<E: RenderElement<VulkanRenderer>>(
    renderer: &mut VulkanRenderer,
    size: Size<i32, Physical>,
    scale: f64,
    transform: Transform,
    elements: &[E],
    clear: Color32F,
) -> Image {
    let mut target = renderer.create_buffer(Fourcc::Argb8888, Size::from((size.w, size.h)))?;
    let mut damage = OutputDamageTracker::new(size, scale, transform);
    {
        let mut framebuffer = renderer.bind(&mut target)?;
        damage
            .render_output(renderer, &mut framebuffer, 0, elements, clear)
            .map_err(rendering_error)?;
    }
    renderer.read_target(&target)
}

fn announce(session: &ExtImageCopyCaptureSessionV1, data: &SessionData, size: Size<i32, Physical>) {
    if let Ok(mut announced) = data.announced.lock() {
        *announced = Some(size);
    }
    session.buffer_size(size.w.unsigned_abs(), size.h.unsigned_abs());
    for format in SHM_FORMATS {
        session.shm_format(format);
    }
    session.done();
}

fn write_into(
    buffer: &WlBuffer,
    size: Size<i32, Physical>,
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
            let target = state
                .screens
                .iter()
                .find(|screen| screen.output.owns(&output))
                .map_or(Target::Nothing, |screen| Target::Output(screen.name()));
            data_init.init(source, SourceData { target });
        }
    }
}

impl GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ExtForeignToplevelImageCaptureSourceManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        is_shell(&client)
    }
}

impl Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
            source,
            toplevel_handle,
        } = request
        {
            let target = ForeignToplevelHandle::from_resource(&toplevel_handle)
                .filter(|handle| !handle.is_closed())
                .map_or(Target::Nothing, |handle| {
                    Target::Window(handle.identifier())
                });
            data_init.init(source, SourceData { target });
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
    target: Target,
    paint_cursors: bool,
    data_init: &mut DataInit<'_, State>,
) {
    let size = state.capture_size(&target);
    let data = SessionData {
        target,
        paint_cursors,
        frames: AtomicUsize::new(0),
        announced: Mutex::new(None),
        delivered: AtomicBool::new(false),
    };
    let session = data_init.init(session, data);
    match (size, session.data::<SessionData>()) {
        (Some(size), Some(data)) => announce(&session, data, size),
        _ => session.stopped(),
    }
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
                let target = source
                    .data::<SourceData>()
                    .map_or(Target::Nothing, |source| source.target.clone());
                let paint_cursors = options.contains(Options::PaintCursors);
                start_session(state, session, target, paint_cursors, data_init);
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
            start_session(state, session, Target::Nothing, false, data_init);
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
                if state.capture_size(&session.target).is_none() {
                    return resource.failed(FailureReason::Stopped);
                }
                if !session.delivered.load(Ordering::Relaxed) {
                    state.needs_redraw = true;
                }
                state.pending_captures.push(PendingCapture {
                    frame: resource.clone(),
                    buffer,
                    session: data.session.clone(),
                });
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
