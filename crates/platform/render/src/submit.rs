use std::collections::VecDeque;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::slice;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use ash::vk;
use smithay::backend::renderer::sync::{Fence, Interrupted, SyncPoint};
use smithay::reexports::rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::{GPU_HANG_NANOS, RenderError, Staging, VulkanRenderer};

const IN_FLIGHT: usize = 3;
const SPARE_STAGING: usize = 2;
const TIMED_FRAMES: usize = 64;

static LAST_SERIAL: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Slot {
    commands: vk::CommandBuffer,
    fence: vk::Fence,
    signal: Option<vk::Semaphore>,
    timestamps: vk::QueryPool,
    serial: u64,
    frame: bool,
    staging: Vec<Staging>,
    sync_file: Option<Arc<OwnedFd>>,
}

pub(crate) enum Garbage {
    Image {
        image: vk::Image,
        view: vk::ImageView,
        memory: Vec<vk::DeviceMemory>,
    },
    Buffer {
        buffer: vk::Buffer,
        memory: vk::DeviceMemory,
    },
}

pub(crate) struct Submissions {
    slots: Vec<Slot>,
    recording: Option<usize>,
    submitted: u64,
    completed: u64,
    spare: Vec<Staging>,
    frame_times: VecDeque<Duration>,
    measured: Vec<Duration>,
}

impl Submissions {
    pub(crate) fn new() -> Self {
        let last = LAST_SERIAL.load(Ordering::Relaxed);
        Self {
            slots: Vec::new(),
            recording: None,
            submitted: last,
            completed: last,
            spare: Vec::new(),
            frame_times: VecDeque::new(),
            measured: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct GpuFence(Arc<OwnedFd>);

fn signalled(fd: &OwnedFd, timeout: Option<&Timespec>) -> bool {
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    poll(&mut fds, timeout).is_ok_and(|ready| ready > 0)
}

impl Fence for GpuFence {
    fn is_signaled(&self) -> bool {
        signalled(&self.0, Some(&Timespec::default()))
    }

    fn wait(&self) -> Result<(), Interrupted> {
        let timeout = Timespec {
            tv_sec: (GPU_HANG_NANOS / 1_000_000_000) as i64,
            tv_nsec: 0,
        };
        match signalled(&self.0, Some(&timeout)) {
            true => Ok(()),
            false => Err(Interrupted),
        }
    }

    fn is_exportable(&self) -> bool {
        true
    }

    fn export(&self) -> Option<OwnedFd> {
        self.0.as_fd().try_clone_to_owned().ok()
    }
}

impl VulkanRenderer {
    fn create_slot(&self) -> Result<Slot, RenderError> {
        // SAFETY: `command_pool` was created from this device with `RESET_COMMAND_BUFFER`; the
        // buffer is freed together with the pool.
        let commands = unsafe {
            self.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }?
        .into_iter()
        .next()
        .ok_or(RenderError::Vulkan(vk::Result::ERROR_UNKNOWN))?;
        let mut slot = Slot {
            commands,
            fence: vk::Fence::null(),
            signal: None,
            timestamps: vk::QueryPool::null(),
            serial: 0,
            frame: false,
            staging: Vec::new(),
            sync_file: None,
        };
        match self.fill_slot(&mut slot) {
            Ok(()) => Ok(slot),
            Err(error) => {
                self.destroy_slot(slot);
                Err(error)
            }
        }
    }

    fn fill_slot(&self, slot: &mut Slot) -> Result<(), RenderError> {
        // SAFETY: the fence is created unsignalled and destroyed only by `destroy_slot`, once the
        // device no longer uses it.
        slot.fence = unsafe {
            self.device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }?;
        if self.external_semaphore_fd.is_some() {
            let mut export = vk::ExportSemaphoreCreateInfo::default()
                .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
            // SAFETY: the device was created with `VK_KHR_external_semaphore_fd` and reported
            // sync file export for binary semaphores; `export` outlives the call.
            slot.signal = Some(unsafe {
                self.device.create_semaphore(
                    &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                    None,
                )
            }?);
        }
        if self.timestamp_period.is_some() {
            // SAFETY: the queue family has valid timestamp bits; the pool is destroyed by
            // `destroy_slot`.
            slot.timestamps = unsafe {
                self.device.create_query_pool(
                    &vk::QueryPoolCreateInfo::default()
                        .query_type(vk::QueryType::TIMESTAMP)
                        .query_count(2),
                    None,
                )
            }?;
        }
        Ok(())
    }

    fn destroy_slot(&self, slot: Slot) {
        // SAFETY: callers pass a slot whose submission finished (or never happened), so the
        // device uses none of its objects; Vulkan ignores the null handles of a partly made slot.
        unsafe {
            self.device.destroy_fence(slot.fence, None);
            if let Some(signal) = slot.signal {
                self.device.destroy_semaphore(signal, None);
            }
            self.device.destroy_query_pool(slot.timestamps, None);
            self.device
                .free_command_buffers(self.command_pool, slice::from_ref(&slot.commands));
        }
        for staging in &slot.staging {
            self.destroy_staging(staging);
        }
    }

    pub(crate) fn begin_commands(&mut self) -> Result<(), RenderError> {
        self.retire();
        let free = self
            .submissions
            .slots
            .iter()
            .position(|slot| slot.serial == 0);
        let index = match free {
            Some(index) => index,
            None if self.submissions.slots.len() < IN_FLIGHT => {
                let slot = self.create_slot()?;
                self.submissions.slots.push(slot);
                self.submissions.slots.len() - 1
            }
            None => {
                let oldest = self
                    .submissions
                    .slots
                    .iter()
                    .map(|slot| slot.serial)
                    .min()
                    .unwrap_or_default();
                self.wait_for(oldest)?;
                self.submissions
                    .slots
                    .iter()
                    .position(|slot| slot.serial == 0)
                    .ok_or(RenderError::Vulkan(vk::Result::ERROR_UNKNOWN))?
            }
        };
        let slot = &self.submissions.slots[index];
        // SAFETY: the slot is free, so its buffer is not pending and its fence is unsignalled;
        // the pool was created with `RESET_COMMAND_BUFFER`, and the query reset and timestamp are
        // recorded outside any rendering.
        unsafe {
            self.device
                .reset_command_buffer(slot.commands, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(
                slot.commands,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            if slot.timestamps != vk::QueryPool::null() {
                self.device
                    .cmd_reset_query_pool(slot.commands, slot.timestamps, 0, 2);
                self.device.cmd_write_timestamp2(
                    slot.commands,
                    vk::PipelineStageFlags2::TOP_OF_PIPE,
                    slot.timestamps,
                    0,
                );
            }
        }
        self.command_buffer = slot.commands;
        self.submissions.recording = Some(index);
        Ok(())
    }

    pub(crate) fn submit(&mut self, frame: bool) -> Result<u64, RenderError> {
        let index = self
            .submissions
            .recording
            .take()
            .ok_or(RenderError::Vulkan(vk::Result::ERROR_UNKNOWN))?;
        let serial = self.submissions.submitted + 1;
        let slot = &mut self.submissions.slots[index];
        let signals: Vec<_> = slot.signal.into_iter().collect();
        let submit = vk::SubmitInfo::default()
            .command_buffers(slice::from_ref(&slot.commands))
            .signal_semaphores(&signals);
        // SAFETY: the renderer is used from one thread, so the queue is externally synchronised;
        // the buffer was begun by `begin_commands` and only recorded into since; the slot's fence
        // is unsignalled and its semaphore unsignalled with no pending signal, as every earlier
        // signal was consumed by exporting it below.
        let submitted = unsafe {
            if slot.timestamps != vk::QueryPool::null() {
                self.device.cmd_write_timestamp2(
                    slot.commands,
                    vk::PipelineStageFlags2::BOTTOM_OF_PIPE,
                    slot.timestamps,
                    1,
                );
            }
            self.device
                .end_command_buffer(slot.commands)
                .and_then(|()| {
                    self.device
                        .queue_submit(self.queue, slice::from_ref(&submit), slot.fence)
                })
        };
        if let Err(error) = submitted {
            if error == vk::Result::ERROR_DEVICE_LOST {
                self.lost.store(true, Ordering::Relaxed);
            }
            return Err(error.into());
        }
        slot.serial = serial;
        slot.frame = frame;
        slot.sync_file = match (slot.signal, &self.external_semaphore_fd) {
            // SAFETY: the semaphore was created exportable as a sync file and the submission just
            // queued signals it; exporting consumes that signal, and the fd is new and ours.
            (Some(signal), Some(external)) => unsafe {
                external
                    .get_semaphore_fd(
                        &vk::SemaphoreGetFdInfoKHR::default()
                            .semaphore(signal)
                            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                    )
                    .ok()
                    .filter(|&fd| fd >= 0)
                    .map(|fd| Arc::new(OwnedFd::from_raw_fd(fd)))
            },
            _ => None,
        };
        self.submissions.submitted = serial;
        LAST_SERIAL.fetch_max(serial, Ordering::Relaxed);
        Ok(serial)
    }

    pub(crate) fn hand_staging_to(&mut self, serial: u64) {
        let staging = std::mem::take(&mut self.staging);
        match self
            .submissions
            .slots
            .iter_mut()
            .find(|slot| slot.serial == serial)
        {
            Some(slot) => slot.staging.extend(staging),
            None => self.spare_staging(staging),
        }
    }

    fn spare_staging(&mut self, staging: Vec<Staging>) {
        let spare = &mut self.submissions.spare;
        spare.extend(
            staging
                .into_iter()
                .map(|staging| Staging { used: 0, ..staging }),
        );
        spare.sort_by_key(|staging| std::cmp::Reverse(staging.capacity));
        let extra = spare.split_off(spare.len().min(SPARE_STAGING));
        for staging in &extra {
            self.destroy_staging(staging);
        }
    }

    pub(crate) fn take_spare_staging(&mut self, bytes: usize) -> Option<Staging> {
        let spare = &mut self.submissions.spare;
        let index = spare.iter().position(|staging| staging.capacity >= bytes)?;
        Some(spare.swap_remove(index))
    }

    pub(crate) fn submit_and_wait(&mut self) -> Result<(), RenderError> {
        let serial = self.submit(false)?;
        self.wait_for(serial)
    }

    pub(crate) fn wait_for(&mut self, serial: u64) -> Result<(), RenderError> {
        let fences: Vec<_> = self
            .submissions
            .slots
            .iter()
            .filter(|slot| slot.serial != 0 && slot.serial <= serial)
            .map(|slot| slot.fence)
            .collect();
        if !fences.is_empty() {
            // SAFETY: every fence belongs to a pending submission of this device.
            let waited = unsafe { self.device.wait_for_fences(&fences, true, GPU_HANG_NANOS) };
            if let Err(error) = waited {
                if matches!(error, vk::Result::ERROR_DEVICE_LOST | vk::Result::TIMEOUT) {
                    self.lost.store(true, Ordering::Relaxed);
                }
                return Err(error.into());
            }
        }
        self.retire();
        Ok(())
    }

    pub fn wait_idle(&mut self) -> Result<(), RenderError> {
        self.wait_for(self.submissions.submitted)
    }

    pub fn retire(&mut self) {
        let mut done = Vec::new();
        for (index, slot) in self.submissions.slots.iter().enumerate() {
            // SAFETY: the fence belongs to this device; a status query has no side effect.
            if slot.serial != 0 && unsafe { self.device.get_fence_status(slot.fence) } == Ok(true) {
                done.push(index);
            }
        }
        for index in done {
            let timed = self.slot_time(index);
            let slot = &mut self.submissions.slots[index];
            // SAFETY: the fence is signalled, so the submission finished and nothing waits on it.
            if let Err(error) = unsafe { self.device.reset_fences(slice::from_ref(&slot.fence)) } {
                tracing::warn!("cannot reuse a finished submission's fence: {error}");
                continue;
            }
            slot.serial = 0;
            slot.sync_file = None;
            let frame = std::mem::take(&mut slot.frame);
            let staging = std::mem::take(&mut slot.staging);
            if let (true, Some(time)) = (frame, timed) {
                let times = &mut self.submissions.frame_times;
                times.push_back(time);
                if times.len() > TIMED_FRAMES {
                    times.pop_front();
                }
                self.submissions.measured.push(time);
            }
            self.spare_staging(staging);
        }
        let pending = self
            .submissions
            .slots
            .iter()
            .map(|slot| slot.serial)
            .filter(|&serial| serial != 0)
            .min();
        self.submissions.completed =
            pending.map_or(self.submissions.submitted, |oldest| oldest - 1);
        let completed = self.submissions.completed;
        let due: Vec<_> = {
            let mut garbage = self.garbage.borrow_mut();
            let (due, kept) = std::mem::take(&mut *garbage)
                .into_iter()
                .partition(|(serial, _)| *serial <= completed);
            *garbage = kept;
            due
        };
        for (_, garbage) in due {
            self.destroy_now(garbage);
        }
    }

    fn slot_time(&self, index: usize) -> Option<Duration> {
        let slot = &self.submissions.slots[index];
        let period = self.timestamp_period?;
        if slot.timestamps == vk::QueryPool::null() {
            return None;
        }
        let mut ticks = [0u64; 2];
        // SAFETY: both queries were written by the finished submission, and `ticks` holds two
        // 64-bit results.
        unsafe {
            self.device.get_query_pool_results(
                slot.timestamps,
                0,
                &mut ticks,
                vk::QueryResultFlags::TYPE_64,
            )
        }
        .ok()?;
        let nanos = ticks[1].saturating_sub(ticks[0]) as f64 * f64::from(period);
        Some(Duration::from_nanos(nanos as u64))
    }

    pub fn submitted(&self) -> u64 {
        self.submissions.submitted
    }

    pub fn completed(&self) -> u64 {
        self.submissions.completed
    }

    pub fn fence_of(&self, serial: u64) -> Option<OwnedFd> {
        self.submissions
            .slots
            .iter()
            .filter(|slot| slot.serial != 0 && slot.serial <= serial)
            .max_by_key(|slot| slot.serial)
            .and_then(|slot| slot.sync_file.as_ref())
            .and_then(|fd| fd.as_fd().try_clone_to_owned().ok())
    }

    pub fn gpu_frame_time(&self) -> Option<Duration> {
        self.submissions
            .frame_times
            .iter()
            .rev()
            .take(4)
            .max()
            .copied()
    }

    pub fn take_gpu_frame_times(&mut self) -> Vec<Duration> {
        std::mem::take(&mut self.submissions.measured)
    }

    pub(crate) fn sync_point(&mut self, serial: u64) -> Result<SyncPoint, RenderError> {
        let file = self
            .submissions
            .slots
            .iter()
            .find(|slot| slot.serial == serial)
            .and_then(|slot| slot.sync_file.clone());
        match file {
            Some(file) => Ok(SyncPoint::from(GpuFence(file))),
            None => {
                self.wait_for(serial)?;
                Ok(SyncPoint::signaled())
            }
        }
    }

    pub(crate) fn discard(&self, garbage: Garbage) {
        let busy = self.submissions.recording.is_some()
            || self.submissions.slots.iter().any(|slot| slot.serial != 0);
        if !busy {
            self.destroy_now(garbage);
            return;
        }
        let serial = self.submissions.submitted + u64::from(self.submissions.recording.is_some());
        self.garbage.borrow_mut().push((serial, garbage));
    }

    fn destroy_now(&self, garbage: Garbage) {
        match garbage {
            Garbage::Image {
                image,
                view,
                memory,
            } => {
                self.forget_sets_of(view);
                // SAFETY: the image is no longer reachable from the renderer's tables and every
                // submission that used it has finished; Vulkan ignores the null handles of a partly
                // built image.
                unsafe {
                    self.device.destroy_image_view(view, None);
                    self.device.destroy_image(image, None);
                    for memory in memory {
                        self.device.free_memory(memory, None);
                    }
                }
            }
            Garbage::Buffer { buffer, memory } => {
                // SAFETY: the buffer is no longer reachable from the renderer's tables and every
                // submission that read it has finished; freeing the memory also unmaps it.
                unsafe {
                    self.device.destroy_buffer(buffer, None);
                    self.device.free_memory(memory, None);
                }
            }
        }
    }

    pub(crate) fn release_submissions(&mut self) {
        for (_, garbage) in std::mem::take(self.garbage.get_mut()) {
            self.destroy_now(garbage);
        }
        for slot in std::mem::take(&mut self.submissions.slots) {
            self.destroy_slot(slot);
        }
        for staging in std::mem::take(&mut self.submissions.spare) {
            self.destroy_staging(&staging);
        }
    }
}
