use std::{fmt, io};

use smithay::backend::allocator::dmabuf::{AsDmabuf, Dmabuf};
use smithay::backend::allocator::dumb::{DumbAllocator, DumbBuffer};
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBuffer, GbmDevice};
use smithay::backend::allocator::{Allocator, Buffer, Format, Fourcc, Modifier};
use smithay::backend::drm::dumb::{DumbFramebuffer, framebuffer_from_dumb_buffer};
use smithay::backend::drm::exporter::{ExportBuffer, ExportFramebuffer};
use smithay::backend::drm::gbm::{
    Error as ClientFramebufferError, GbmFramebuffer, framebuffer_from_bo,
    framebuffer_from_wayland_buffer,
};
use smithay::backend::drm::{DrmAccessError, DrmDeviceFd, DrmNode, Framebuffer};
use smithay::reexports::drm::control::framebuffer;
use smithay::utils::{Buffer as BufferCoords, Size};
use smithay::wayland::dmabuf::get_dmabuf;

#[derive(Debug)]
pub(crate) enum ScanoutAllocator {
    Rendered(GbmAllocator<DrmDeviceFd>),
    Copied(DumbAllocator),
}

#[derive(Debug)]
pub(crate) enum ScanoutBuffer {
    Rendered(GbmBuffer),
    Copied(DumbBuffer),
}

impl Allocator for ScanoutAllocator {
    type Buffer = ScanoutBuffer;
    type Error = io::Error;

    fn create_buffer(
        &mut self,
        width: u32,
        height: u32,
        fourcc: Fourcc,
        modifiers: &[Modifier],
    ) -> io::Result<ScanoutBuffer> {
        match self {
            Self::Rendered(allocator) => allocator
                .create_buffer(width, height, fourcc, modifiers)
                .map(ScanoutBuffer::Rendered),
            Self::Copied(allocator) => allocator
                .create_buffer(width, height, fourcc, modifiers)
                .map(ScanoutBuffer::Copied),
        }
    }
}

impl Buffer for ScanoutBuffer {
    fn size(&self) -> Size<i32, BufferCoords> {
        match self {
            Self::Rendered(buffer) => buffer.size(),
            Self::Copied(buffer) => buffer.size(),
        }
    }

    fn format(&self) -> Format {
        match self {
            Self::Rendered(buffer) => buffer.format(),
            Self::Copied(buffer) => buffer.format(),
        }
    }
}

impl AsDmabuf for ScanoutBuffer {
    type Error = io::Error;

    fn export(&self) -> io::Result<Dmabuf> {
        match self {
            Self::Rendered(buffer) => buffer.export().map_err(io::Error::other),
            Self::Copied(buffer) => buffer.export(),
        }
    }
}

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
    Rendered(GbmFramebuffer),
    Client(GbmFramebuffer),
}

impl AsRef<framebuffer::Handle> for ScanoutFramebuffer {
    fn as_ref(&self) -> &framebuffer::Handle {
        match self {
            Self::Composed(framebuffer) => framebuffer.as_ref(),
            Self::Rendered(framebuffer) | Self::Client(framebuffer) => framebuffer.as_ref(),
        }
    }
}

impl Framebuffer for ScanoutFramebuffer {
    fn format(&self) -> smithay::backend::allocator::Format {
        match self {
            Self::Composed(framebuffer) => framebuffer.format(),
            Self::Rendered(framebuffer) | Self::Client(framebuffer) => framebuffer.format(),
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

impl ExportFramebuffer<ScanoutBuffer> for ScanoutExporter {
    type Framebuffer = ScanoutFramebuffer;
    type Error = ScanoutExportError;

    fn add_framebuffer(
        &self,
        drm: &DrmDeviceFd,
        buffer: ExportBuffer<'_, ScanoutBuffer>,
        use_opaque: bool,
    ) -> Result<Option<ScanoutFramebuffer>, ScanoutExportError> {
        match buffer {
            ExportBuffer::Allocator(ScanoutBuffer::Copied(buffer)) => {
                framebuffer_from_dumb_buffer(drm, buffer, use_opaque)
                    .map(|framebuffer| Some(ScanoutFramebuffer::Composed(framebuffer)))
                    .map_err(ScanoutExportError::Composed)
            }
            ExportBuffer::Allocator(ScanoutBuffer::Rendered(buffer)) => {
                framebuffer_from_bo(drm, buffer, use_opaque)
                    .map(|framebuffer| Some(ScanoutFramebuffer::Rendered(framebuffer)))
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

    fn can_add_framebuffer(&self, buffer: &ExportBuffer<'_, ScanoutBuffer>) -> bool {
        match buffer {
            ExportBuffer::Allocator(_) => true,
            ExportBuffer::Wayland(_) if self.gbm.is_none() => false,
            ExportBuffer::Wayland(buffer) => get_dmabuf(buffer)
                .ok()
                .is_some_and(|dmabuf| dmabuf.node() == Some(self.node)),
        }
    }
}
