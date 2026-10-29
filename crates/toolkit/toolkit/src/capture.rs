use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::protocol::wl_shm::Format;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
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

use crate::window::{State, Window, WindowError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Capture {
    pub(crate) sources: Option<ExtOutputImageCaptureSourceManagerV1>,
    pub(crate) manager: Option<ExtImageCopyCaptureManagerV1>,
    size: Option<(u32, u32)>,
    formats: Vec<Format>,
    constraints_done: bool,
    stopped: bool,
    outcome: Option<Result<(), u32>>,
}

const FORMAT: Format = Format::Xrgb8888;

impl Window {
    pub fn capture_screen(&mut self) -> Result<ScreenImage, WindowError> {
        let missing = WindowError::Missing("ext_image_copy_capture_manager_v1");
        let handle = self.queue.handle();
        let capture = &self.state.capture;
        let (Some(sources), Some(manager), Some(output), Some(shm)) = (
            capture.sources.as_ref(),
            capture.manager.as_ref(),
            self.state.lock.output.as_ref(),
            self.state.shm.clone(),
        ) else {
            return Err(missing);
        };
        let source = sources.create_source(output, &handle, ());
        let session = manager.create_session(&source, Options::empty(), &handle, ());
        self.state.capture.size = None;
        self.state.capture.formats.clear();
        self.state.capture.constraints_done = false;
        self.state.capture.stopped = false;
        self.state.capture.outcome = None;
        while !self.state.capture.constraints_done && !self.state.capture.stopped {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        let constraints = &self.state.capture;
        let size = constraints.size.filter(|_| !constraints.stopped);
        let Some((width, height)) = size.filter(|_| constraints.formats.contains(&FORMAT)) else {
            session.destroy();
            source.destroy();
            return Err(missing);
        };
        let stride = width.saturating_mul(4);
        let length = u64::from(stride) * u64::from(height);
        let file = File::from(
            memfd_create("unbound-screen-capture", MemfdFlags::CLOEXEC).map_err(io::Error::from)?,
        );
        file.set_len(length)?;
        let pool_length =
            i32::try_from(length).map_err(|_| io::Error::other("screen too large"))?;
        let side =
            |value: u32| i32::try_from(value).map_err(|_| io::Error::other("screen too large"));
        let pool = shm.create_pool(file.as_fd(), pool_length, &handle, ());
        let buffer = pool.create_buffer(
            0,
            side(width)?,
            side(height)?,
            side(stride)?,
            FORMAT,
            &handle,
            Arc::new(AtomicBool::new(false)),
        );
        pool.destroy();
        let frame = session.create_frame(&handle, ());
        frame.attach_buffer(&buffer);
        frame.damage_buffer(0, 0, side(width)?, side(height)?);
        frame.capture();
        while self.state.capture.outcome.is_none() {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        frame.destroy();
        buffer.destroy();
        session.destroy();
        source.destroy();
        if let Some(Err(reason)) = self.state.capture.outcome {
            return Err(WindowError::CaptureFailed(reason));
        }
        let mut pixels = vec![0; usize::try_from(length).map_err(io::Error::other)?];
        file.read_exact_at(&mut pixels, 0)?;
        Ok(ScreenImage {
            width,
            height,
            rgba: xrgb_to_rgba(pixels),
        })
    }
}

fn xrgb_to_rgba(mut pixels: Vec<u8>) -> Vec<u8> {
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let [blue, green, red, _] = *pixel;
        *pixel = [red, green, blue, u8::MAX];
    }
    pixels
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
        let capture = &mut state.capture;
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                capture.size = Some((width, height));
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => capture.formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => capture.constraints_done = true,
            ext_image_copy_capture_session_v1::Event::Stopped => capture.stopped = true,
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
            ext_image_copy_capture_frame_v1::Event::Ready => state.capture.outcome = Some(Ok(())),
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                state.capture.outcome = Some(Err(reason.into()));
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ExtImageCaptureSourceV1);
delegate_noop!(State: ExtImageCopyCaptureManagerV1);

#[cfg(test)]
mod tests {
    use super::xrgb_to_rgba;

    #[test]
    fn little_endian_xrgb_becomes_opaque_rgba() {
        let xrgb = vec![0x30, 0x20, 0x10, 0x00, 0xff, 0x00, 0x80, 0x7f];

        assert_eq!(
            xrgb_to_rgba(xrgb),
            vec![0x10, 0x20, 0x30, 0xff, 0x80, 0x00, 0xff, 0xff]
        );
    }
}
