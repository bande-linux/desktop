use std::fmt;

use smithay::backend::allocator::dumb::DumbBuffer;
use smithay::backend::allocator::gbm::GbmDevice;
use smithay::backend::drm::dumb::{DumbFramebuffer, framebuffer_from_dumb_buffer};
use smithay::backend::drm::exporter::{ExportBuffer, ExportFramebuffer};
use smithay::backend::drm::gbm::{
    Error as ClientFramebufferError, GbmFramebuffer, framebuffer_from_wayland_buffer,
};
use smithay::backend::drm::{DrmAccessError, DrmDeviceFd, DrmNode, Framebuffer};
use smithay::reexports::drm::control::framebuffer;
use smithay::wayland::dmabuf::get_dmabuf;

#[derive(Debug)]
pub(crate) struct ScanoutExporter {
    gbm: Option<GbmDevice<DrmDeviceFd>>,
    node: DrmNode,
}

impl ScanoutExporter {
    pub(crate) fn new(gbm: Option<GbmDevice<DrmDeviceFd>>, node: DrmNode) -> Self {
        Self { gbm, node }
    }
}

#[derive(Debug)]
pub(crate) enum ScanoutFramebuffer {
    Composed(DumbFramebuffer),
    Client(GbmFramebuffer),
}

impl AsRef<framebuffer::Handle> for ScanoutFramebuffer {
    fn as_ref(&self) -> &framebuffer::Handle {
        match self {
            Self::Composed(framebuffer) => framebuffer.as_ref(),
            Self::Client(framebuffer) => framebuffer.as_ref(),
        }
    }
}

impl Framebuffer for ScanoutFramebuffer {
    fn format(&self) -> smithay::backend::allocator::Format {
        match self {
            Self::Composed(framebuffer) => framebuffer.format(),
            Self::Client(framebuffer) => framebuffer.format(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum ScanoutExportError {
    Composed(DrmAccessError),
    Client(ClientFramebufferError),
}

impl fmt::Display for ScanoutExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Composed(error) => write!(f, "cannot add a framebuffer for a frame: {error}"),
            Self::Client(error) => {
                write!(f, "cannot add a framebuffer for a client buffer: {error}")
            }
        }
    }
}

impl std::error::Error for ScanoutExportError {}

impl ExportFramebuffer<DumbBuffer> for ScanoutExporter {
    type Framebuffer = ScanoutFramebuffer;
    type Error = ScanoutExportError;

    fn add_framebuffer(
        &self,
        drm: &DrmDeviceFd,
        buffer: ExportBuffer<'_, DumbBuffer>,
        use_opaque: bool,
    ) -> Result<Option<ScanoutFramebuffer>, ScanoutExportError> {
        match buffer {
            ExportBuffer::Allocator(buffer) => {
                framebuffer_from_dumb_buffer(drm, buffer, use_opaque)
                    .map(|framebuffer| Some(ScanoutFramebuffer::Composed(framebuffer)))
                    .map_err(ScanoutExportError::Composed)
            }
            ExportBuffer::Wayland(buffer) => {
                let Some(gbm) = &self.gbm else {
                    return Ok(None);
                };
                framebuffer_from_wayland_buffer(drm, gbm, buffer, use_opaque)
                    .map(|framebuffer| framebuffer.map(ScanoutFramebuffer::Client))
                    .map_err(ScanoutExportError::Client)
            }
        }
    }

    fn can_add_framebuffer(&self, buffer: &ExportBuffer<'_, DumbBuffer>) -> bool {
        match buffer {
            ExportBuffer::Allocator(_) => true,
            ExportBuffer::Wayland(_) if self.gbm.is_none() => false,
            ExportBuffer::Wayland(buffer) => get_dmabuf(buffer)
                .ok()
                .is_some_and(|dmabuf| dmabuf.node() == Some(self.node)),
        }
    }
}
