#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;
use std::process::{Child, Command};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use common::{Desktop, OUTPUT, WAIT, WINDOW};
use pipewire as pw;
use pw::spa;
use pw::spa::param::ParamType;
use pw::spa::param::format::{FormatProperties, MediaSubtype, MediaType};
use pw::spa::param::video::{VideoFormat, VideoInfoRaw};
use pw::spa::pod::Pod;
use pw::spa::utils::{Fraction, Rectangle, SpaTypes};
use pw::stream::{StreamBox, StreamFlags};
use tempfile::TempDir;
use unbound_screenshot::cast::Cast;
use unbound_screenshot::engine::Source;
use unbound_test_bus::SessionBus;

const NO_HARDWARE: &str = r#"
wireplumber.profiles = {
    main = {
        monitor.alsa = disabled
        monitor.alsa-midi = disabled
        monitor.bluez = disabled
        monitor.bluez-midi = disabled
        monitor.libcamera = disabled
        monitor.v4l2 = disabled
    }
}
"#;

struct PipeWire {
    daemons: Vec<Child>,
    _bus: SessionBus,
    _home: TempDir,
}

impl PipeWire {
    fn start() -> Self {
        let home = TempDir::new().expect("home");
        let runtime = home.path().join("run");
        let config = home.path().join("config");
        std::fs::create_dir_all(&runtime).expect("runtime dir");
        std::fs::create_dir_all(config.join("wireplumber/wireplumber.conf.d"))
            .expect("wireplumber config dir");
        std::fs::write(
            config.join("wireplumber/wireplumber.conf.d/no-hardware.conf"),
            NO_HARDWARE,
        )
        .expect("no hardware");
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", &runtime);
            std::env::set_var("XDG_CONFIG_HOME", &config);
            std::env::set_var("XDG_STATE_HOME", home.path().join("state"));
            std::env::remove_var("PIPEWIRE_REMOTE");
        }
        let bus = SessionBus::start().expect("session bus");
        let daemon = |program: &str| {
            Command::new(program)
                .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
                .spawn()
                .unwrap_or_else(|error| panic!("{program}: {error}"))
        };
        let mut daemons = vec![daemon("pipewire")];
        wait_for(&runtime.join("pipewire-0"));
        daemons.push(daemon("wireplumber"));
        Self {
            daemons,
            _bus: bus,
            _home: home,
        }
    }
}

impl Drop for PipeWire {
    fn drop(&mut self) {
        for daemon in self.daemons.iter_mut().rev() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
    }
}

fn wait_for(socket: &Path) {
    let deadline = Instant::now() + WAIT;
    while !socket.exists() {
        assert!(
            Instant::now() < deadline,
            "PipeWire never opened {socket:?}"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

#[derive(Debug)]
struct Received {
    width: u32,
    height: u32,
    pixel: [u8; 4],
}

fn consume(node: u32, at: (u32, u32), received: mpsc::Sender<Received>) {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None).expect("main loop");
    let context = pw::context::ContextRc::new(&main_loop, None).expect("context");
    let core = context.connect_rc(None).expect("core");
    let stream = StreamBox::new(
        &core,
        "screen-cast-consumer",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .expect("stream");
    let quit = main_loop.clone();
    let _listener = stream
        .add_local_listener_with_user_data(VideoInfoRaw::new())
        .param_changed(|_, format, id, param| {
            if let Some(param) = param
                && id == ParamType::Format.as_raw()
            {
                format.parse(param).expect("video format");
            }
        })
        .process(move |stream, format| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let size = format.size();
            let data = &mut buffer.datas_mut()[0];
            let stride = data.chunk().stride() as u32;
            let Some(bytes) = data.data() else {
                return;
            };
            let offset = (at.1 * stride + at.0 * 4) as usize;
            let pixel = [
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ];
            let _ = received.send(Received {
                width: size.width,
                height: size.height,
                pixel,
            });
            quit.quit();
        })
        .register()
        .expect("listener");
    let format = spa::pod::object!(
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
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 320,
                height: 240
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 30, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );
    let bytes = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(format),
    )
    .expect("pod")
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&bytes).expect("pod bytes")];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .expect("connect");
    main_loop.run();
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn a_pipewire_consumer_receives_the_shared_screen() {
    let _pipewire = PipeWire::start();
    let mut desktop = Desktop::start();
    let windows = desktop.wait(|engine| engine.windows());
    let frame = windows[0].frame.expect("shown");
    let center = (
        (frame.x + WINDOW.0 / 2) as u32,
        (frame.y + WINDOW.1 / 2) as u32,
    );
    let (cast, started) = Cast::new("test cast").expect("cast");
    desktop.engine.stream(1, Source::Screen, false, cast.sink());

    let started = desktop
        .wait(move |_| Cast::started(&started))
        .expect("the stream started");
    let (sender, received) = mpsc::channel();
    thread::spawn(move || consume(started.node_id, center, sender));
    let deadline = Instant::now() + WAIT;
    let frame = loop {
        if let Ok(frame) = received.try_recv() {
            break frame;
        }
        assert!(Instant::now() < deadline, "no frame reached the consumer");
        desktop.step();
    };

    assert_eq!(
        (started.width, started.height),
        (OUTPUT.0 as u32, OUTPUT.1 as u32)
    );
    assert_eq!(
        (frame.width, frame.height),
        (OUTPUT.0 as u32, OUTPUT.1 as u32)
    );
    assert_eq!(&frame.pixel[..3], &[0x00, 0x00, 0xff]);
}
