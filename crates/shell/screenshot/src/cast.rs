use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use pw::spa::param::ParamType;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, PropertyFlags, Value};
use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Rectangle, SpaTypes};
use pw::stream::{StreamFlags, StreamListener, StreamRc, StreamState};
use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};

const BYTES_PER_PIXEL: u32 = 4;
const STRIDE_ALIGNMENT: u32 = 64;
const MOST_FRAMES_PER_SECOND: u32 = 60;
const BUFFERS: i32 = 4;
const START_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_millis(50);
const UDMABUF_DEVICE: &str = "/dev/udmabuf";
const UDMABUF_CREATE: libc::c_ulong = 0x4018_7542;
const UDMABUF_CLOEXEC: u32 = 1;

#[repr(C)]
struct UdmabufCreate {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

enum Message {
    Size(u32, u32),
    Present(u32),
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Started {
    pub node_id: u32,
    pub width: u32,
    pub height: u32,
}

pub struct SharedBuffer {
    pub slot: u32,
    pub memory: File,
    pub dmabuf: Option<OwnedFd>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

#[derive(Default)]
pub struct PoolChanges {
    pub added: Vec<SharedBuffer>,
    pub removed: Vec<u32>,
    pub restarted: bool,
}

#[derive(Default)]
struct Shelf {
    changes: PoolChanges,
    free: VecDeque<u32>,
}

type Notify = Box<dyn Fn() + Send>;

#[derive(Clone)]
pub struct CastPool {
    messages: pw::channel::Sender<Message>,
    shelf: Arc<Mutex<Shelf>>,
    notify: Arc<Mutex<Option<Notify>>>,
}

impl CastPool {
    pub fn on_change(&self, notify: Notify) {
        *self.notify.lock().unwrap_or_else(PoisonError::into_inner) = Some(notify);
    }

    pub fn set_size(&self, width: u32, height: u32) {
        let _ = self.messages.send(Message::Size(width, height));
    }

    pub fn present(&self, slot: u32) {
        let _ = self.messages.send(Message::Present(slot));
    }

    pub fn changes(&self) -> PoolChanges {
        std::mem::take(&mut lock(&self.shelf).changes)
    }

    pub fn next_free(&self) -> Option<u32> {
        lock(&self.shelf).free.pop_front()
    }

    pub fn give_back(&self, slot: u32) {
        lock(&self.shelf).free.push_front(slot);
    }

    fn changed(&self) {
        if let Some(notify) = &*self.notify.lock().unwrap_or_else(PoisonError::into_inner) {
            notify();
        }
    }
}

fn lock(shelf: &Mutex<Shelf>) -> std::sync::MutexGuard<'_, Shelf> {
    shelf.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Cast {
    pool: CastPool,
}

impl Cast {
    pub fn new(name: &str) -> io::Result<(Self, mpsc::Receiver<Started>)> {
        let (messages, receiver) = pw::channel::channel();
        let (started, started_receiver) = mpsc::channel();
        let pool = CastPool {
            messages,
            shelf: Arc::default(),
            notify: Arc::default(),
        };
        let served = pool.clone();
        let name = name.to_owned();
        thread::Builder::new().name("cast".into()).spawn(move || {
            if let Err(error) = serve(&name, receiver, served, started) {
                tracing::warn!("cannot share the screen through PipeWire: {error}");
            }
        })?;
        Ok((Self { pool }, started_receiver))
    }

    pub fn pool(&self) -> CastPool {
        self.pool.clone()
    }

    pub fn started(receiver: &mpsc::Receiver<Started>) -> Option<Started> {
        receiver.recv_timeout(START_TIMEOUT).ok()
    }
}

impl Drop for Cast {
    fn drop(&mut self) {
        let _ = self.pool.messages.send(Message::Stop);
    }
}

fn pod_bytes(object: Object) -> Option<Vec<u8>> {
    spa::pod::serialize::PodSerializer::serialize(
        io::Cursor::new(Vec::new()),
        &Value::Object(object),
    )
    .ok()
    .map(|(cursor, _)| cursor.into_inner())
}

fn stride_of(width: u32) -> u32 {
    (width * BYTES_PER_PIXEL).next_multiple_of(STRIDE_ALIGNMENT)
}

fn format(width: u32, height: u32, dmabuf: bool) -> Object {
    let mut object = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(FormatProperties::VideoFormat, Id, VideoFormat::BGRA),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Rectangle,
            Rectangle { width, height }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Fraction,
            Fraction { num: 0, denom: 1 }
        ),
        spa::pod::property!(
            FormatProperties::VideoMaxFramerate,
            Choice,
            Range,
            Fraction,
            Fraction {
                num: MOST_FRAMES_PER_SECOND,
                denom: 1
            },
            Fraction { num: 1, denom: 1 },
            Fraction {
                num: MOST_FRAMES_PER_SECOND,
                denom: 1
            }
        ),
    );
    if dmabuf {
        object.properties.push(Property {
            key: FormatProperties::VideoModifier.as_raw(),
            flags: PropertyFlags::MANDATORY,
            value: Value::Long(0),
        });
    }
    object
}

fn formats(width: u32, height: u32) -> Vec<Vec<u8>> {
    let dmabuf_possible = File::options()
        .read(true)
        .write(true)
        .open(UDMABUF_DEVICE)
        .is_ok();
    dmabuf_possible
        .then(|| format(width, height, true))
        .into_iter()
        .chain([format(width, height, false)])
        .filter_map(pod_bytes)
        .collect()
}

fn buffers(width: u32, height: u32, dmabuf: bool) -> Object {
    let stride = stride_of(width);
    let kind = match dmabuf {
        true => spa::sys::SPA_DATA_DmaBuf,
        false => spa::sys::SPA_DATA_MemFd,
    };
    let int = |key: u32, value: i32| Property::new(key, Value::Int(value));
    Object {
        type_: SpaTypes::ObjectParamBuffers.as_raw(),
        id: ParamType::Buffers.as_raw(),
        properties: vec![
            Property::new(
                spa::sys::SPA_PARAM_BUFFERS_buffers,
                Value::Choice(ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: BUFFERS,
                        min: 2,
                        max: 8,
                    },
                ))),
            ),
            int(spa::sys::SPA_PARAM_BUFFERS_blocks, 1),
            int(
                spa::sys::SPA_PARAM_BUFFERS_size,
                i32::try_from(stride * height).unwrap_or(i32::MAX),
            ),
            int(
                spa::sys::SPA_PARAM_BUFFERS_stride,
                i32::try_from(stride).unwrap_or(i32::MAX),
            ),
            int(spa::sys::SPA_PARAM_BUFFERS_dataType, 1 << kind),
        ],
    }
}

fn header() -> Object {
    Object {
        type_: SpaTypes::ObjectParamMeta.as_raw(),
        id: ParamType::Meta.as_raw(),
        properties: vec![
            Property::new(
                spa::sys::SPA_PARAM_META_type,
                Value::Id(spa::utils::Id(spa::sys::SPA_META_Header)),
            ),
            Property::new(
                spa::sys::SPA_PARAM_META_size,
                Value::Int(std::mem::size_of::<spa::sys::spa_meta_header>() as i32),
            ),
        ],
    }
}

fn udmabuf_of(memory: &File, length: u64) -> io::Result<OwnedFd> {
    let device = File::options()
        .read(true)
        .write(true)
        .open(UDMABUF_DEVICE)?;
    let request = UdmabufCreate {
        memfd: memory.as_raw_fd() as u32,
        flags: UDMABUF_CLOEXEC,
        offset: 0,
        size: length,
    };
    let fd = unsafe { libc::ioctl(device.as_raw_fd(), UDMABUF_CREATE as _, &request) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn shared_memory(length: u64) -> io::Result<(File, Option<OwnedFd>)> {
    let page = rustix::param::page_size() as u64;
    let rounded = length.next_multiple_of(page);
    let memory = File::from(memfd_create(
        "bande-cast",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?);
    memory.set_len(rounded)?;
    fcntl_add_seals(memory.as_fd(), SealFlags::SHRINK)?;
    let dmabuf = udmabuf_of(&memory, rounded)
        .inspect_err(|error| tracing::debug!("no dmabuf for the screen cast: {error}"))
        .ok();
    Ok((memory, dmabuf))
}

struct Slot {
    buffer: *mut pw::sys::pw_buffer,
    _memory: File,
    _dmabuf: Option<OwnedFd>,
}

struct Producer {
    stream: Option<StreamRc>,
    listener: Option<StreamListener<()>>,
    size: (u32, u32),
    dmabuf: bool,
    started: Option<Sender<Started>>,
    pool: CastPool,
    slots: HashMap<u32, Slot>,
    next_slot: u32,
    ready: Vec<u32>,
    has_streamed: bool,
}

impl Producer {
    fn slot_of(&self, buffer: *mut pw::sys::pw_buffer) -> Option<u32> {
        self.slots
            .iter()
            .find(|(_, slot)| slot.buffer == buffer)
            .map(|(id, _)| *id)
    }
}

fn presentation_time() -> i64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    now.tv_sec
        .saturating_mul(1_000_000_000)
        .saturating_add(now.tv_nsec)
}

fn add_buffer(producer: &Rc<RefCell<Producer>>, buffer: *mut pw::sys::pw_buffer) {
    let Ok(mut shared) = producer.try_borrow_mut() else {
        return;
    };
    let (width, height) = shared.size;
    let stride = stride_of(width);
    let length = u64::from(stride) * u64::from(height);
    let (memory, dmabuf) = match shared_memory(length) {
        Ok(allocated) => allocated,
        Err(error) => {
            tracing::warn!("cannot allocate a screen cast buffer: {error}");
            return;
        }
    };
    let fd = match (shared.dmabuf, &dmabuf) {
        (true, Some(dmabuf)) => dmabuf.as_raw_fd(),
        _ => memory.as_raw_fd(),
    };
    let kind = match shared.dmabuf {
        true => spa::sys::SPA_DATA_DmaBuf,
        false => spa::sys::SPA_DATA_MemFd,
    };
    unsafe {
        let spa = (*buffer).buffer;
        if spa.is_null() || (*spa).n_datas == 0 || (*spa).datas.is_null() {
            return;
        }
        let data = &mut *(*spa).datas;
        data.type_ = kind;
        data.flags = spa::sys::SPA_DATA_FLAG_READWRITE;
        data.fd = i64::from(fd);
        data.mapoffset = 0;
        data.maxsize = u32::try_from(length).unwrap_or(u32::MAX);
        data.data = std::ptr::null_mut();
    }
    let slot = shared.next_slot;
    shared.next_slot += 1;
    let handed = (
        memory.try_clone(),
        dmabuf.as_ref().map(OwnedFd::try_clone).transpose(),
    );
    let (Ok(handed_memory), Ok(handed_dmabuf)) = handed else {
        return;
    };
    shared.slots.insert(
        slot,
        Slot {
            buffer,
            _memory: memory,
            _dmabuf: dmabuf,
        },
    );
    lock(&shared.pool.shelf).changes.added.push(SharedBuffer {
        slot,
        memory: handed_memory,
        dmabuf: handed_dmabuf,
        width,
        height,
        stride,
    });
    shared.pool.changed();
}

fn remove_buffer(producer: &Rc<RefCell<Producer>>, buffer: *mut pw::sys::pw_buffer) {
    let Ok(mut shared) = producer.try_borrow_mut() else {
        return;
    };
    let Some(slot) = shared.slot_of(buffer) else {
        return;
    };
    shared.slots.remove(&slot);
    shared.ready.retain(|ready| *ready != slot);
    {
        let mut shelf = lock(&shared.pool.shelf);
        shelf.free.retain(|free| *free != slot);
        shelf.changes.removed.push(slot);
    }
    shared.pool.changed();
}

fn describe(buffer: *mut pw::sys::pw_buffer, stride: u32, length: u32) {
    unsafe {
        let spa = (*buffer).buffer;
        if spa.is_null() || (*spa).n_datas == 0 || (*spa).datas.is_null() {
            return;
        }
        let data = &mut *(*spa).datas;
        if !data.chunk.is_null() {
            let chunk = &mut *data.chunk;
            chunk.offset = 0;
            chunk.stride = i32::try_from(stride).unwrap_or(i32::MAX);
            chunk.size = length;
        }
        for index in 0..(*spa).n_metas as usize {
            let meta = &*(*spa).metas.add(index);
            if meta.type_ == spa::sys::SPA_META_Header
                && meta.size as usize >= std::mem::size_of::<spa::sys::spa_meta_header>()
                && !meta.data.is_null()
            {
                let header = &mut *meta.data.cast::<spa::sys::spa_meta_header>();
                header.flags = 0;
                header.offset = 0;
                header.pts = presentation_time();
                header.dts_offset = 0;
                header.seq = header.seq.wrapping_add(1);
            }
        }
    }
}

fn process(producer: &Rc<RefCell<Producer>>) {
    let Ok(mut shared) = producer.try_borrow_mut() else {
        return;
    };
    let Some(stream) = shared.stream.clone() else {
        return;
    };
    let raw = stream.as_raw_ptr();
    let stride = stride_of(shared.size.0);
    let length = stride * shared.size.1;
    for slot in std::mem::take(&mut shared.ready) {
        if let Some(entry) = shared.slots.get(&slot) {
            describe(entry.buffer, stride, length);
            unsafe { pw::sys::pw_stream_queue_buffer(raw, entry.buffer) };
        }
    }
    let mut freed = false;
    loop {
        let buffer = unsafe { pw::sys::pw_stream_dequeue_buffer(raw) };
        if buffer.is_null() {
            break;
        }
        if let Some(slot) = shared.slot_of(buffer) {
            lock(&shared.pool.shelf).free.push_back(slot);
            freed = true;
        }
    }
    if freed {
        shared.pool.changed();
    }
}

fn wake(producer: &Rc<RefCell<Producer>>) {
    let stream = producer.try_borrow().ok().and_then(|shared| {
        shared
            .stream
            .clone()
            .filter(|stream| stream.state() == StreamState::Streaming)
    });
    if let Some(stream) = stream
        && let Err(error) = stream.trigger_process()
    {
        tracing::warn!("cannot hand a frame to PipeWire: {error}");
    }
}

fn connect(
    core: &pw::core::CoreRc,
    name: &str,
    producer: &Rc<RefCell<Producer>>,
    (width, height): (u32, u32),
) -> Result<(), pw::Error> {
    let stream = StreamRc::new(
        core.clone(),
        name,
        properties! {
            *pw::keys::MEDIA_CLASS => "Video/Source",
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
            *pw::keys::NODE_NAME => "bande-screen-cast",
            *pw::keys::NODE_DESCRIPTION => name,
        },
    )?;
    let changed = Rc::downgrade(producer);
    let negotiated = Rc::downgrade(producer);
    let processed = Rc::downgrade(producer);
    let added = Rc::downgrade(producer);
    let removed = Rc::downgrade(producer);
    let listener = stream
        .add_local_listener_with_user_data(())
        .state_changed(move |stream, _, _, state| {
            let Some(producer) = changed.upgrade() else {
                return;
            };
            if matches!(state, StreamState::Paused | StreamState::Streaming) {
                let started = producer
                    .try_borrow_mut()
                    .ok()
                    .and_then(|mut shared| Some((shared.started.take()?, shared.size)));
                if let Some((started, (width, height))) = started {
                    let _ = started.send(Started {
                        node_id: stream.node_id(),
                        width,
                        height,
                    });
                }
            }
            if state == StreamState::Streaming {
                if let Ok(mut shared) = producer.try_borrow_mut()
                    && std::mem::replace(&mut shared.has_streamed, true)
                {
                    lock(&shared.pool.shelf).changes.restarted = true;
                    shared.pool.changed();
                }
                wake(&producer);
            }
        })
        .param_changed(move |stream, _, id, param| {
            let (Some(param), Some(producer)) = (param, negotiated.upgrade()) else {
                return;
            };
            if id != ParamType::Format.as_raw() {
                return;
            }
            let mut info = VideoInfoRaw::new();
            if info.parse(param).is_err() {
                return;
            }
            let size = info.size();
            let dmabuf = info.flags().bits() & spa::sys::SPA_VIDEO_FLAG_MODIFIER != 0;
            if let Ok(mut shared) = producer.try_borrow_mut() {
                shared.size = (size.width, size.height);
                shared.dmabuf = dmabuf;
            }
            let bytes: Vec<Vec<u8>> = [buffers(size.width, size.height, dmabuf), header()]
                .into_iter()
                .filter_map(pod_bytes)
                .collect();
            let mut params: Vec<&Pod> = bytes
                .iter()
                .filter_map(|bytes| Pod::from_bytes(bytes))
                .collect();
            if let Err(error) = stream.update_params(&mut params) {
                tracing::warn!("cannot set the screen cast buffers: {error}");
            }
            wake(&producer);
        })
        .add_buffer(move |_, _, buffer| {
            if let Some(producer) = added.upgrade() {
                add_buffer(&producer, buffer);
            }
        })
        .remove_buffer(move |_, _, buffer| {
            if let Some(producer) = removed.upgrade() {
                remove_buffer(&producer, buffer);
            }
        })
        .process(move |_, _| {
            if let Some(producer) = processed.upgrade() {
                process(&producer);
            }
        })
        .register()?;
    {
        let mut shared = producer.borrow_mut();
        shared.size = (width, height);
        shared.stream = Some(stream.clone());
        shared.listener = Some(listener);
    }
    let bytes = formats(width, height);
    let mut params: Vec<&Pod> = bytes
        .iter()
        .filter_map(|bytes| Pod::from_bytes(bytes))
        .collect();
    stream.connect(
        spa::utils::Direction::Output,
        None,
        StreamFlags::DRIVER | StreamFlags::ALLOC_BUFFERS,
        &mut params,
    )
}

fn resize(producer: &Rc<RefCell<Producer>>, size: (u32, u32)) {
    let stream = {
        let mut shared = producer.borrow_mut();
        shared.size = size;
        shared.stream.clone()
    };
    let Some(stream) = stream else {
        return;
    };
    let bytes = formats(size.0, size.1);
    let mut params: Vec<&Pod> = bytes
        .iter()
        .filter_map(|bytes| Pod::from_bytes(bytes))
        .collect();
    if let Err(error) = stream.update_params(&mut params) {
        tracing::warn!("cannot resize the screen cast: {error}");
    }
}

fn serve(
    name: &str,
    messages: pw::channel::Receiver<Message>,
    pool: CastPool,
    started: Sender<Started>,
) -> Result<(), pw::Error> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let producer = Rc::new(RefCell::new(Producer {
        stream: None,
        listener: None,
        size: (0, 0),
        dmabuf: false,
        started: Some(started),
        pool,
        slots: HashMap::new(),
        next_slot: 0,
        ready: Vec::new(),
        has_streamed: false,
    }));
    let quit = main_loop.clone();
    let fed = Rc::clone(&producer);
    let name = name.to_owned();
    let _messages = messages.attach(main_loop.loop_(), move |message| match message {
        Message::Stop => quit.quit(),
        Message::Size(width, height) => {
            let size = (width, height);
            let streaming = fed.borrow().stream.is_some();
            if !streaming {
                if let Err(error) = connect(&core, &name, &fed, size) {
                    tracing::warn!("cannot create the screen cast stream: {error}");
                    quit.quit();
                }
            } else if fed.borrow().size != size {
                resize(&fed, size);
            }
        }
        Message::Present(slot) => {
            fed.borrow_mut().ready.push(slot);
            wake(&fed);
        }
    });
    let retried = Rc::clone(&producer);
    let retry = main_loop.loop_().add_timer(move |_| {
        let waiting = retried
            .try_borrow()
            .is_ok_and(|shared| lock(&shared.pool.shelf).free.is_empty());
        if waiting {
            wake(&retried);
        }
    });
    let _ = retry.update_timer(Some(RETRY), Some(RETRY));
    main_loop.run();
    let mut producer = producer.borrow_mut();
    if let Some(stream) = producer.stream.take() {
        let _ = stream.disconnect();
    }
    producer.listener = None;
    Ok(())
}
