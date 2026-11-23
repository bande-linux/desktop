use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bande_graphics::Region;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_surface::WlSurface;

const MAX_STALE_REGIONS: usize = 64;
const MAX_BUFFERS: usize = 3;

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
        idle: impl Fn(&T) -> bool,
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
        let free = |buffer: &Pooled<T>| buffer.size == size && !buffer.busy.load(Ordering::Acquire);
        let idle_buffer = self
            .buffers
            .iter()
            .position(|buffer| free(buffer) && idle(&buffer.contents));
        let room = self
            .buffers
            .iter()
            .filter(|buffer| buffer.size == size)
            .count()
            < MAX_BUFFERS;
        let reused = match (idle_buffer, room) {
            (Some(index), _) => Some(index),
            (None, true) => None,
            (None, false) => self.buffers.iter().position(free),
        };
        let index = match reused {
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

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::os::unix::net::UnixStream;

    use wayland_client::{Connection, Proxy};

    use super::*;

    struct Frames {
        pool: BufferPool<u32>,
        busy: Vec<Arc<AtomicBool>>,
        reading: Vec<u32>,
        connection: Connection,
        _server: UnixStream,
    }

    fn region(x: u32) -> Region {
        Region {
            x,
            y: 0,
            width: 1,
            height: 1,
        }
    }

    impl Frames {
        fn new() -> Self {
            let (client, server) = UnixStream::pair().unwrap();
            Self {
                pool: BufferPool::default(),
                busy: Vec::new(),
                reading: Vec::new(),
                connection: Connection::from_socket(client).unwrap(),
                _server: server,
            }
        }

        fn show(
            &mut self,
            size: (u32, u32),
            damage: Option<&[Region]>,
        ) -> (u32, Option<Vec<Region>>) {
            let backend = self.connection.backend().downgrade();
            let created = self.busy.len() as u32;
            let mut made = None;
            let (buffer, stale) = self
                .pool
                .next(
                    size,
                    damage,
                    |buffer| !self.reading.contains(buffer),
                    |busy| {
                        made = Some(Arc::clone(&busy));
                        Ok::<_, Infallible>((created, WlBuffer::inert(backend)))
                    },
                )
                .unwrap();
            buffer.busy.store(true, Ordering::Release);
            let shown = (buffer.contents, stale);
            self.busy.extend(made);
            shown
        }

        fn release(&self, buffer: u32) {
            self.busy[buffer as usize].store(false, Ordering::Release);
        }
    }

    #[test]
    fn a_reused_buffer_redraws_what_changed_since_it_was_last_shown() {
        let mut frames = Frames::new();
        let size = (8, 8);
        assert_eq!(frames.show(size, None), (0, None));
        assert_eq!(frames.show(size, Some(&[region(1)])), (1, None));
        frames.release(0);
        assert_eq!(
            frames.show(size, Some(&[region(2)])),
            (0, Some(vec![region(1), region(2)]))
        );
        frames.release(1);
        assert_eq!(
            frames.show(size, Some(&[region(3)])),
            (1, Some(vec![region(2), region(3)]))
        );
    }

    #[test]
    fn a_third_buffer_comes_only_while_the_compositor_holds_both() {
        let mut frames = Frames::new();
        let size = (8, 8);
        frames.show(size, None);
        frames.show(size, None);
        assert_eq!(frames.show(size, None), (2, None));
        frames.release(0);
        assert_eq!(frames.show(size, Some(&[region(1)])).0, 0);
    }

    #[test]
    fn too_many_changes_or_a_new_size_redraw_everything() {
        let mut frames = Frames::new();
        let size = (8, 8);
        frames.show(size, None);
        let many: Vec<Region> = (0..MAX_STALE_REGIONS as u32 + 1).map(region).collect();
        assert_eq!(frames.show(size, Some(&many)), (1, None));
        frames.release(0);
        assert_eq!(frames.show(size, Some(&[region(1)])), (0, None));

        frames.release(1);
        assert_eq!(frames.show((9, 9), Some(&[region(1)])), (2, None));
        assert_eq!(frames.pool.buffers.len(), 2, "the busy old buffer is kept");
        frames.release(0);
        frames.show((9, 9), Some(&[region(2)]));
        assert!(
            frames
                .pool
                .buffers
                .iter()
                .all(|buffer| buffer.size == (9, 9))
        );
    }

    #[test]
    fn a_buffer_the_compositor_still_reads_is_skipped_up_to_a_third_buffer() {
        let mut frames = Frames::new();
        let size = (8, 8);
        frames.show(size, None);
        frames.show(size, None);
        frames.release(0);
        frames.reading.push(0);
        assert_eq!(
            frames.show(size, None).0,
            2,
            "a third buffer instead of waiting"
        );
        frames.release(1);
        frames.reading.push(1);
        assert_eq!(
            frames.show(size, None).0,
            0,
            "no fourth buffer: the oldest released one"
        );
    }
}
