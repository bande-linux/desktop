use std::io;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use bande_screen_sharing::rfb::{Frame, Screen};
use pipewire as pw;
use pw::spa;
use pw::spa::param::ParamType;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::pod::{Pod, Value};
use pw::spa::utils::{Rectangle, SpaTypes};
use pw::stream::{StreamFlags, StreamListener, StreamRc, StreamState};
use zbus::blocking::Connection;
use zbus::zvariant::OwnedFd;

use crate::input::RemoteInput;

pub const SCREENSHOT: &str = "org.bande.Screenshot";
pub const SCREENSHOT_PATH: &str = "/org/bande/Screenshot";
pub const SCREENSHOT_INTERFACE: &str = "org.bande.Screenshot";
const BYTES_PER_PIXEL: usize = 4;
const LARGEST: u32 = 16384;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The screen shown to viewers while any of them holds it: a PipeWire stream from the
/// screenshot tool and input through the compositor.
pub struct Capture {
    pub node: u32,
    pub screen: Arc<Screen>,
    pub input: Arc<RemoteInput>,
    bus: Connection,
    stop: pw::channel::Sender<()>,
}

impl Capture {
    pub fn start(bus: &Connection) -> io::Result<Self> {
        let (node, width, height, x, y, socket): (u32, u32, u32, i32, i32, OwnedFd) = bus
            .call_method(
                Some(SCREENSHOT),
                SCREENSHOT_PATH,
                Some(SCREENSHOT_INTERFACE),
                "StartViewerStream",
                &(),
            )
            .and_then(|reply| reply.body().deserialize())
            .map_err(io::Error::other)?;
        let socket = UnixStream::from(std::os::fd::OwnedFd::from(socket));
        let input = Arc::new(RemoteInput::connect(socket, (width, height), (x, y))?);
        let screen = Arc::new(Screen::default());
        let (stop, stopped) = pw::channel::channel();
        let (connected, connection) = mpsc::channel();
        let shown = Arc::clone(&screen);
        thread::Builder::new()
            .name("pipewire".into())
            .spawn(move || {
                if let Err(error) = consume(node, &shown, stopped, connected) {
                    tracing::warn!("cannot receive the screen: {error}");
                }
                shown.end();
            })?;
        let capture = Self {
            node,
            screen,
            input,
            bus: bus.clone(),
            stop,
        };
        match connection.recv_timeout(CONNECT_TIMEOUT) {
            Ok(()) => Ok(capture),
            Err(_) => Err(io::Error::other("the screen stream did not start")),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        self.screen.end();
        if let Err(error) = self.bus.call_method(
            Some(SCREENSHOT),
            SCREENSHOT_PATH,
            Some(SCREENSHOT_INTERFACE),
            "StopViewerStream",
            &self.node,
        ) {
            tracing::warn!("cannot stop showing the screen: {error}");
        }
    }
}

fn format_pod() -> Option<Vec<u8>> {
    let object = spa::pod::object!(
        SpaTypes::ObjectParamFormat,
        ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRA,
            VideoFormat::BGRA,
            VideoFormat::BGRx
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 1920,
                height: 1080
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: LARGEST,
                height: LARGEST
            }
        ),
    );
    spa::pod::serialize::PodSerializer::serialize(
        io::Cursor::new(Vec::new()),
        &Value::Object(object),
    )
    .ok()
    .map(|(cursor, _)| cursor.into_inner())
}

fn frame_of(data: &mut spa::buffer::Data, size: (u32, u32)) -> Option<Frame> {
    let (width, height) = size;
    let chunk = data.chunk();
    let offset = chunk.offset() as usize;
    let stride = usize::try_from(chunk.stride())
        .ok()
        .filter(|stride| *stride >= width as usize * BYTES_PER_PIXEL)?;
    let bytes = data.data()?.get(offset..)?;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        let line = bytes.get(start..start + width as usize * BYTES_PER_PIXEL)?;
        pixels.extend(
            line.as_chunks::<4>()
                .0
                .iter()
                .map(|&[blue, green, red, _]| u32::from_be_bytes([0, red, green, blue])),
        );
    }
    Some(Frame {
        width,
        height,
        pixels,
    })
}

fn consume(
    node: u32,
    screen: &Arc<Screen>,
    stop: pw::channel::Receiver<()>,
    connected: mpsc::Sender<()>,
) -> Result<(), pw::Error> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let mut properties = pw::properties::PropertiesBox::new();
    properties.insert(*pw::keys::MEDIA_TYPE, "Video");
    properties.insert(*pw::keys::MEDIA_CATEGORY, "Capture");
    properties.insert(*pw::keys::MEDIA_ROLE, "Screen");
    let stream = StreamRc::new(core, "bande-screen-sharing", properties)?;
    let shown = Arc::clone(screen);
    let ended = Arc::clone(screen);
    let quit = main_loop.clone();
    let _listener: StreamListener<(u32, u32)> = stream
        .add_local_listener_with_user_data((0, 0))
        .param_changed(|_, size, id, param| {
            let Some(param) = param.filter(|_| id == ParamType::Format.as_raw()) else {
                return;
            };
            let mut info = VideoInfoRaw::new();
            if info.parse(param).is_ok() {
                let negotiated = info.size();
                *size = (negotiated.width, negotiated.height);
            }
        })
        .state_changed(move |_, _, _, state| {
            if let StreamState::Error(error) = state {
                tracing::warn!("the screen stream failed: {error}");
                ended.end();
                quit.quit();
            }
        })
        .process(move |stream, size| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(data) = buffer.datas_mut().first_mut() else {
                return;
            };
            if size.0 > 0
                && let Some(frame) = frame_of(data, *size)
            {
                shown.show(frame);
            }
        })
        .register()?;
    let bytes = format_pod().ok_or(pw::Error::CreationFailed)?;
    let pod = Pod::from_bytes(&bytes).ok_or(pw::Error::CreationFailed)?;
    stream.connect(
        spa::utils::Direction::Input,
        Some(node),
        StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
        &mut [pod],
    )?;
    let _ = connected.send(());
    let stopping = main_loop.clone();
    let _stop = stop.attach(main_loop.loop_(), move |()| stopping.quit());
    main_loop.run();
    if let Err(error) = stream.disconnect() {
        tracing::debug!("cannot disconnect the screen stream: {error}");
    }
    Ok(())
}
