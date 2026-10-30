use std::cell::RefCell;
use std::io;
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
use pw::spa::pod::{ChoiceValue, Object, Pod, Property, Value};
use pw::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Rectangle, SpaTypes};
use pw::stream::{StreamFlags, StreamListener, StreamRc, StreamState};

use crate::engine::FrameSink;

const BYTES_PER_PIXEL: u32 = 4;
const MOST_FRAMES_PER_SECOND: u32 = 60;
const BUFFERS: i32 = 4;
const START_TIMEOUT: Duration = Duration::from_secs(5);

type Picture = (u32, u32, Vec<u8>);

enum Message {
    Picture,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Started {
    pub node_id: u32,
    pub width: u32,
    pub height: u32,
}

pub struct Cast {
    messages: pw::channel::Sender<Message>,
    latest: Arc<Mutex<Option<Picture>>>,
}

impl Cast {
    pub fn new(name: &str) -> io::Result<(Self, mpsc::Receiver<Started>)> {
        let (messages, receiver) = pw::channel::channel();
        let (started, started_receiver) = mpsc::channel();
        let latest = Arc::new(Mutex::new(None));
        let pictures = Arc::clone(&latest);
        let name = name.to_owned();
        thread::Builder::new().name("cast".into()).spawn(move || {
            if let Err(error) = serve(&name, receiver, pictures, started) {
                tracing::warn!("cannot share the screen through PipeWire: {error}");
            }
        })?;
        Ok((Self { messages, latest }, started_receiver))
    }

    pub fn sink(&self) -> FrameSink {
        let messages = self.messages.clone();
        let latest = Arc::clone(&self.latest);
        Box::new(move |width, height, bgra| {
            let mut slot = latest.lock().unwrap_or_else(PoisonError::into_inner);
            let woken = slot.is_some();
            match slot.as_mut() {
                Some((w, h, pixels)) if (*w, *h) == (width, height) => {
                    pixels.copy_from_slice(bgra);
                }
                _ => *slot = Some((width, height, bgra.to_vec())),
            }
            drop(slot);
            if !woken {
                let _ = messages.send(Message::Picture);
            }
        })
    }

    pub fn started(receiver: &mpsc::Receiver<Started>) -> Option<Started> {
        receiver.recv_timeout(START_TIMEOUT).ok()
    }
}

impl Drop for Cast {
    fn drop(&mut self) {
        let _ = self.messages.send(Message::Stop);
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

fn format(width: u32, height: u32) -> Object {
    spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA
        ),
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
    )
}

fn buffers(width: u32, height: u32) -> Object {
    let stride = width * BYTES_PER_PIXEL;
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
            int(
                spa::sys::SPA_PARAM_BUFFERS_dataType,
                (1 << spa::sys::SPA_DATA_MemFd) | (1 << spa::sys::SPA_DATA_MemPtr),
            ),
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

struct Producer {
    stream: Option<StreamRc>,
    listener: Option<StreamListener<()>>,
    size: (u32, u32),
    started: Option<Sender<Started>>,
    latest: Arc<Mutex<Option<Picture>>>,
    pending: Option<Picture>,
}

impl Producer {
    fn take_latest(&mut self) {
        if let Some(picture) = self
            .latest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            self.pending = Some(picture);
        }
    }
}

fn push(producer: &Rc<RefCell<Producer>>) {
    let Ok(mut shared) = producer.try_borrow_mut() else {
        return;
    };
    let Some(stream) = shared.stream.clone() else {
        return;
    };
    if stream.state() != StreamState::Streaming {
        return;
    }
    let size = shared.size;
    let Some((width, height, pixels)) = shared.pending.take() else {
        return;
    };
    drop(shared);
    if (width, height) != size {
        return;
    }
    let Some(mut buffer) = stream.dequeue_buffer() else {
        if let Ok(mut shared) = producer.try_borrow_mut() {
            shared.pending.get_or_insert((width, height, pixels));
        }
        return;
    };
    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    let stride = width * BYTES_PER_PIXEL;
    if let Some(memory) = data.data() {
        let length = memory.len().min(pixels.len());
        memory[..length].copy_from_slice(&pixels[..length]);
        let chunk = data.chunk_mut();
        *chunk.offset_mut() = 0;
        *chunk.stride_mut() = i32::try_from(stride).unwrap_or(i32::MAX);
        *chunk.size_mut() = u32::try_from(length).unwrap_or(u32::MAX);
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
            *pw::keys::NODE_NAME => "unbound-screen-cast",
            *pw::keys::NODE_DESCRIPTION => name,
        },
    )?;
    let changed = Rc::downgrade(producer);
    let negotiated = Rc::downgrade(producer);
    let processed = Rc::downgrade(producer);
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
                push(&producer);
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
            if let Ok(mut shared) = producer.try_borrow_mut() {
                shared.size = (size.width, size.height);
            }
            let objects = [buffers(size.width, size.height), header()];
            let bytes: Vec<Vec<u8>> = objects.into_iter().filter_map(pod_bytes).collect();
            let mut params: Vec<&Pod> = bytes
                .iter()
                .filter_map(|bytes| Pod::from_bytes(bytes))
                .collect();
            if let Err(error) = stream.update_params(&mut params) {
                tracing::warn!("cannot set the screen cast buffers: {error}");
            }
        })
        .process(move |_, _| {
            if let Some(producer) = processed.upgrade() {
                push(&producer);
            }
        })
        .register()?;
    {
        let mut shared = producer.borrow_mut();
        shared.size = (width, height);
        shared.stream = Some(stream.clone());
        shared.listener = Some(listener);
    }
    let bytes = pod_bytes(format(width, height)).ok_or(pw::Error::CreationFailed)?;
    let mut params = [Pod::from_bytes(&bytes).ok_or(pw::Error::CreationFailed)?];
    stream.connect(
        spa::utils::Direction::Output,
        None,
        StreamFlags::DRIVER | StreamFlags::MAP_BUFFERS,
        &mut params,
    )
}

fn resize(producer: &Rc<RefCell<Producer>>, size: (u32, u32)) {
    let stream = {
        let mut shared = producer.borrow_mut();
        shared.size = size;
        shared.stream.clone()
    };
    let (Some(stream), Some(bytes)) = (stream, pod_bytes(format(size.0, size.1))) else {
        return;
    };
    let Some(pod) = Pod::from_bytes(&bytes) else {
        return;
    };
    if let Err(error) = stream.update_params(&mut [pod]) {
        tracing::warn!("cannot resize the screen cast: {error}");
    }
}

fn serve(
    name: &str,
    messages: pw::channel::Receiver<Message>,
    latest: Arc<Mutex<Option<Picture>>>,
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
        started: Some(started),
        latest,
        pending: None,
    }));
    let quit = main_loop.clone();
    let fed = Rc::clone(&producer);
    let name = name.to_owned();
    let _messages = messages.attach(main_loop.loop_(), move |message| match message {
        Message::Stop => quit.quit(),
        Message::Picture => {
            fed.borrow_mut().take_latest();
            let size = fed
                .borrow()
                .pending
                .as_ref()
                .map(|(width, height, _)| (*width, *height));
            let Some(size) = size else {
                return;
            };
            let streaming = fed.borrow().stream.is_some();
            if !streaming {
                if let Err(error) = connect(&core, &name, &fed, size) {
                    tracing::warn!("cannot create the screen cast stream: {error}");
                    quit.quit();
                }
            } else if fed.borrow().size != size {
                resize(&fed, size);
            }
            push(&fed);
        }
    });
    main_loop.run();
    let mut producer = producer.borrow_mut();
    if let Some(stream) = producer.stream.take() {
        let _ = stream.disconnect();
    }
    producer.listener = None;
    Ok(())
}
