use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bande_graphics::Region;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_surface::WlSurface;

const MAX_STALE_REGIONS: usize = 64;

pub(super) struct Pooled<T> {
    pub(super) contents: T,
    buffer: WlBuffer,
    size: (u32, u32),
    busy: Arc<AtomicBool>,
    stale: Option<Vec<Region>>,
}

pub(super) struct BufferPool<T> {
    buffers: Vec<Pooled<T>>,
}

impl<T> Default for BufferPool<T> {
    fn default() -> Self {
        Self {
            buffers: Vec::new(),
        }
    }
}

impl<T> BufferPool<T> {
    pub(super) fn next<E>(
        &mut self,
        size: (u32, u32),
        damage: Option<&[Region]>,
        create: impl FnOnce(Arc<AtomicBool>) -> Result<(T, WlBuffer), E>,
    ) -> Result<(&mut Pooled<T>, Option<Vec<Region>>), E> {
        self.buffers
            .retain(|buffer| buffer.size == size || buffer.busy.load(Ordering::Acquire));
        for buffer in &mut self.buffers {
            buffer.stale = match (buffer.stale.take(), damage) {
                (Some(mut stale), Some(regions))
                    if stale.len() + regions.len() <= MAX_STALE_REGIONS =>
                {
                    stale.extend_from_slice(regions);
                    Some(stale)
                }
                _ => None,
            };
        }
        let free = self
            .buffers
            .iter()
            .position(|buffer| buffer.size == size && !buffer.busy.load(Ordering::Acquire));
        let index = match free {
            Some(index) => index,
            None => {
                let busy = Arc::new(AtomicBool::new(false));
                let (contents, buffer) = create(Arc::clone(&busy))?;
                self.buffers.push(Pooled {
                    contents,
                    buffer,
                    size,
                    busy,
                    stale: None,
                });
                self.buffers.len() - 1
            }
        };
        let buffer = &mut self.buffers[index];
        let stale = buffer.stale.replace(Vec::new());
        Ok((buffer, stale))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.buffers.clear();
    }
}

impl<T> Pooled<T> {
    pub(super) fn show(&self, surface: &WlSurface, damage: Option<&[Region]>) {
        self.busy.store(true, Ordering::Release);
        surface.attach(Some(&self.buffer), 0, 0);
        match damage {
            Some(regions) => {
                for region in regions {
                    surface.damage_buffer(
                        clamp_i32(region.x),
                        clamp_i32(region.y),
                        clamp_i32(region.width),
                        clamp_i32(region.height),
                    );
                }
            }
            None => surface.damage_buffer(0, 0, clamp_i32(self.size.0), clamp_i32(self.size.1)),
        }
        surface.commit();
    }
}

impl<T> Drop for Pooled<T> {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

pub(super) fn clamp_i32(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
