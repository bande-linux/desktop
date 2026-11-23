use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use bande_graphics::{CpuRenderer, Region, Scene, pixels};
use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_surface::WlSurface;

use super::pool::{BufferPool, clamp_i32};
use super::{State, WindowError};

#[derive(Clone, Copy)]
pub(crate) struct Target<'a> {
    pub(crate) shm: &'a WlShm,
    pub(crate) handle: &'a QueueHandle<State>,
    pub(crate) surface: &'a WlSurface,
}

#[derive(Default)]
pub(crate) struct ShmTarget {
    buffers: BufferPool<File>,
    bgra: Vec<u8>,
    scratch: Vec<u8>,
    size: (u32, u32),
}

fn copy_swizzled(source: &[u8], target: &mut [u8]) {
    for (to, from) in target
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(source.as_chunks::<4>().0)
    {
        *to = [from[2], from[1], from[0], from[3]];
    }
}

impl ShmTarget {
    pub(crate) fn present(
        &mut self,
        cpu: &mut CpuRenderer,
        target: Target<'_>,
        scene: &Scene,
        scale: f64,
        damage: Option<&[Region]>,
    ) -> Result<(), WindowError> {
        let damage = self.render(cpu, scene, scale, damage)?;
        self.commit(target, damage, false)
    }

    pub(crate) fn render<'a>(
        &mut self,
        cpu: &mut CpuRenderer,
        scene: &Scene,
        scale: f64,
        damage: Option<&'a [Region]>,
    ) -> Result<Option<&'a [Region]>, WindowError> {
        let (width, height) = (pixels(scene.width, scale), pixels(scene.height, scale));
        let bytes = width as usize * height as usize * 4;
        let damage = damage.filter(|_| self.bgra.len() == bytes);
        match damage {
            Some(regions) => self.render_regions(cpu, scene, scale, width, regions)?,
            None => {
                self.bgra.resize(bytes, 0);
                cpu.render_into(scene, scale, &mut self.bgra)?;
                for pixel in self.bgra.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
            }
        }
        self.size = (width, height);
        Ok(damage)
    }

    pub(crate) fn present_moved(
        &mut self,
        target: Target<'_>,
        alpha: u32,
        (dx, dy): (i32, i32),
    ) -> Result<(), WindowError> {
        if alpha >= 256 && (dx, dy) == (0, 0) {
            return self.commit(target, None, false);
        }
        let (width, height) = (self.size.0 as usize, self.size.1 as usize);
        let stride = width * 4;
        let columns = width.saturating_sub(dx.unsigned_abs() as usize) * 4;
        let (from_x, to_x) = match dx >= 0 {
            true => (0, dx as usize * 4),
            false => (dx.unsigned_abs() as usize * 4, 0),
        };
        self.scratch.clear();
        self.scratch.resize(self.bgra.len(), 0);
        for (row, target) in self.scratch.chunks_exact_mut(stride).enumerate() {
            let Some(source) = row
                .checked_add_signed(-(dy as isize))
                .filter(|source| *source < height)
                .and_then(|source| self.bgra.chunks_exact(stride).nth(source))
            else {
                continue;
            };
            for (to, from) in target[to_x..to_x + columns]
                .iter_mut()
                .zip(&source[from_x..from_x + columns])
            {
                *to = ((u32::from(*from) * alpha) >> 8) as u8;
            }
        }
        self.commit(target, None, true)
    }

    fn commit(
        &mut self,
        target: Target<'_>,
        damage: Option<&[Region]>,
        faded: bool,
    ) -> Result<(), WindowError> {
        let (width, height) = self.size;
        let (buffer, stale) = self.buffers.next(self.size, damage, |busy| {
            create_buffer(target.shm, target.handle, width, height, busy)
        })?;
        let pixels = match faded {
            true => &self.scratch,
            false => &self.bgra,
        };
        match stale {
            None => buffer.contents.write_all_at(pixels, 0)?,
            Some(stale) => {
                let stride = width as usize * 4;
                for region in stale {
                    for row in region.y..region.y + region.height {
                        let start = row as usize * stride + region.x as usize * 4;
                        let end = start + region.width as usize * 4;
                        buffer
                            .contents
                            .write_all_at(&pixels[start..end], start as u64)?;
                    }
                }
            }
        }
        buffer.show(target.surface, damage);
        Ok(())
    }

    fn render_regions(
        &mut self,
        cpu: &mut CpuRenderer,
        scene: &Scene,
        scale: f64,
        width: u32,
        regions: &[Region],
    ) -> Result<(), WindowError> {
        let stride = width as usize * 4;
        for region in regions {
            let row_bytes = region.width as usize * 4;
            self.scratch.clear();
            self.scratch.resize(row_bytes * region.height as usize, 0);
            cpu.render_region(scene, scale, *region, &mut self.scratch)?;
            for (row, source) in self.scratch.chunks_exact(row_bytes).enumerate() {
                let start = (region.y as usize + row) * stride + region.x as usize * 4;
                copy_swizzled(source, &mut self.bgra[start..start + row_bytes]);
            }
        }
        Ok(())
    }

    pub(super) fn is_holding(&self) -> bool {
        !self.buffers.is_empty() || self.bgra.capacity() > 0
    }

    pub(crate) fn destroy(&mut self) {
        self.buffers.clear();
        self.bgra = Vec::new();
        self.scratch = Vec::new();
    }
}

fn create_buffer(
    shm: &WlShm,
    handle: &QueueHandle<State>,
    width: u32,
    height: u32,
    busy: Arc<AtomicBool>,
) -> Result<(File, WlBuffer), WindowError> {
    let stride = clamp_i32(width.saturating_mul(4));
    let length = u64::from(width) * u64::from(height) * 4;
    let file =
        File::from(memfd_create("bande-toolkit", MemfdFlags::CLOEXEC).map_err(io::Error::from)?);
    file.set_len(length)?;
    let pool = shm.create_pool(
        file.as_fd(),
        clamp_i32(u32::try_from(length).unwrap_or(u32::MAX)),
        handle,
        (),
    );
    let buffer = pool.create_buffer(
        0,
        clamp_i32(width),
        clamp_i32(height),
        stride,
        wl_shm::Format::Argb8888,
        handle,
        busy,
    );
    pool.destroy();
    Ok((file, buffer))
}
