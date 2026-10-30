#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;
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
use unbound_inbox::Inbox;
use unbound_screenshot::area::Region;
use unbound_screenshot::cast::Cast;
use unbound_screenshot::engine::Source;
use unbound_screenshot::record::{Container, Recording};
use unbound_screenshot::service::{self, Casting, UiRequest};
use unbound_test_bus::SessionBus;
use xdg_desktop_portal_unbound::capture::ScreenshotTool;
use xdg_desktop_portal_unbound::screencast::ScreenCast;
use xdg_desktop_portal_unbound::{BUS_NAME, OBJECT_PATH};
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

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
    bus: SessionBus,
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
            bus,
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

fn consume(node: u32, remote: Option<OwnedFd>, at: (u32, u32), received: mpsc::Sender<Received>) {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None).expect("main loop");
    let context = pw::context::ContextRc::new(&main_loop, None).expect("context");
    let core = match remote {
        Some(fd) => context.connect_fd_rc(fd, None),
        None => context.connect_rc(None),
    }
    .expect("core");
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
#[ignore = "needs pipewire, wireplumber and gstreamer; ci/vm runs it in the guest"]
fn a_recording_of_an_area_is_saved_as_a_playable_file() {
    let _pipewire = PipeWire::start();
    let container = Container::available().expect("gstreamer with pipewiresrc and an encoder");
    let mut desktop = Desktop::start();
    let (cast, started) = Cast::new("test recording").expect("cast");
    desktop.engine.stream(1, Source::Screen, false, cast.sink());
    let started = desktop
        .wait(move |_| Cast::started(&started))
        .expect("the stream started");
    let output = TempDir::new().expect("output dir");
    let path = output
        .path()
        .join(format!("Recording.{}", container.extension()));
    let recording = Recording::start(
        started.node_id,
        (started.width, started.height),
        Some(Region {
            x: 10,
            y: 10,
            width: 101,
            height: 81,
        }),
        container,
        path.clone(),
    )
    .expect("recorder");
    desktop.pump(Duration::from_secs(2));

    let finishing = thread::spawn(move || recording.finish());
    let deadline = Instant::now() + WAIT;
    while !finishing.is_finished() {
        assert!(Instant::now() < deadline, "the recorder never finished");
        desktop.step();
    }
    let saved = finishing.join().expect("finish").expect("a recording");

    let bytes = std::fs::read(&saved).expect("the file");
    match container {
        Container::Mp4 => assert_eq!(&bytes[4..8], b"ftyp"),
        Container::WebM => assert_eq!(&bytes[..4], &[0x1a, 0x45, 0xdf, 0xa3]),
    }
    assert!(bytes.len() > 1000, "only {} bytes", bytes.len());
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
    thread::spawn(move || consume(started.node_id, None, center, sender));
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

type Results = HashMap<String, OwnedValue>;

const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREEN_CAST: &str = "org.freedesktop.portal.ScreenCast";

fn portal_request<B: serde::Serialize + zbus::zvariant::DynamicType>(
    client: &Connection,
    token: &str,
    method: &str,
    body: &B,
) -> Results {
    let sender = client
        .unique_name()
        .expect("unique name")
        .trim_start_matches(':')
        .replace('.', "_");
    let path = format!("{PORTAL_PATH}/request/{sender}/{token}");
    let responses = MessageIterator::for_match_rule(
        MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.portal.Request")
            .expect("interface")
            .member("Response")
            .expect("member")
            .path(path.as_str())
            .expect("path")
            .build(),
        client,
        None,
    )
    .expect("response subscription");
    client
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            PORTAL_PATH,
            Some(SCREEN_CAST),
            method,
            body,
        )
        .unwrap_or_else(|error| panic!("{method}: {error}"));
    let response = responses
        .into_iter()
        .next()
        .expect("a response")
        .expect("a message");
    let (code, results): (u32, Results) = response.body().deserialize().expect("(ua{sv})");
    assert_eq!(code, 0, "{method} answered {code}: {results:?}");
    results
}

fn share_through_the_portal(client: Connection) -> (u32, OwnedFd) {
    let options = |token: &str| HashMap::from([("handle_token", Value::from(token.to_owned()))]);
    let mut create = options("create");
    create.insert("session_handle_token", Value::from("session"));
    let created = portal_request(&client, "create", "CreateSession", &(create,));
    let session = String::try_from(created["session_handle"].try_clone().expect("clone"))
        .expect("session handle");
    let session = ObjectPath::try_from(session).expect("session path");
    let mut select = options("select");
    select.insert("types", Value::from(1u32));
    select.insert("cursor_mode", Value::from(2u32));
    portal_request(&client, "select", "SelectSources", &(&session, select));
    let started = portal_request(&client, "start", "Start", &(&session, "", options("start")));
    let streams: Vec<(u32, Results)> = started["streams"]
        .try_clone()
        .expect("clone")
        .try_into()
        .expect("a(ua{sv})");
    let remote: zbus::zvariant::OwnedFd = client
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            PORTAL_PATH,
            Some(SCREEN_CAST),
            "OpenPipeWireRemote",
            &(&session, HashMap::<&str, Value>::new()),
        )
        .expect("OpenPipeWireRemote")
        .body()
        .deserialize()
        .expect("h");
    (streams[0].0, OwnedFd::from(remote))
}

#[test]
#[ignore = "needs pipewire, wireplumber and xdg-desktop-portal; ci/vm runs it in the guest"]
fn an_app_shares_the_screen_through_xdg_desktop_portal() {
    let pipewire = PipeWire::start();
    unsafe {
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", pipewire.bus.address());
    }
    let mut desktop = Desktop::start();
    let windows = desktop.wait(|engine| engine.windows());
    let frame = windows[0].frame.expect("shown");
    let center = (
        (frame.x + WINDOW.0 / 2) as u32,
        (frame.y + WINDOW.1 / 2) as u32,
    );
    let mut requests = Inbox::new().expect("inbox");
    let casting = Casting::new(desktop.engine.clone());
    let _service = service::serve(&casting, requests.outbox()).expect("capture service");
    let backend = Connection::session().expect("portal connection");
    let tool = Arc::new(ScreenshotTool::new(
        Connection::session().expect("tool connection"),
    ));
    backend
        .object_server()
        .at(OBJECT_PATH, ScreenCast::new(tool))
        .expect("screen cast backend");
    backend.request_name(BUS_NAME).expect("backend name");
    let config = TempDir::new().expect("portal config");
    std::fs::create_dir_all(config.path().join("portals")).expect("portals dir");
    std::fs::write(
        config.path().join("portals/unbound.portal"),
        include_str!("../../../services/portal/data/unbound.portal"),
    )
    .expect("portal file");
    std::fs::write(
        config.path().join("portals/unbound-portals.conf"),
        include_str!("../../../services/portal/data/unbound-portals.conf"),
    )
    .expect("portals conf");
    let mut frontend = Command::new("/usr/libexec/xdg-desktop-portal")
        .env("XDG_CURRENT_DESKTOP", "Unbound")
        .env("XDG_DESKTOP_PORTAL_DIR", config.path().join("portals"))
        .env("DBUS_SESSION_BUS_ADDRESS", pipewire.bus.address())
        .spawn()
        .expect("xdg-desktop-portal");
    let client = Connection::session().expect("app connection");
    let bus = zbus::blocking::fdo::DBusProxy::new(&client).expect("bus");
    let started = Instant::now() + WAIT;
    while !bus
        .name_has_owner("org.freedesktop.portal.Desktop".try_into().expect("name"))
        .expect("NameHasOwner")
    {
        assert!(Instant::now() < started, "xdg-desktop-portal never started");
        desktop.step();
    }
    let sharing = thread::spawn(move || share_through_the_portal(client));
    let mut picked = None;
    let deadline = Instant::now() + WAIT * 3;
    while !sharing.is_finished() {
        assert!(
            Instant::now() < deadline,
            "the portal never started sharing"
        );
        for request in requests.drain() {
            if let UiRequest::Pick { choices, reply, .. } = request {
                picked = Some(
                    choices
                        .iter()
                        .map(|choice| choice.id.clone())
                        .collect::<Vec<_>>(),
                );
                let first = choices.first().map(|choice| choice.id.clone());
                let _ = reply.send(first.map(|id| vec![id]));
            }
        }
        desktop.step();
    }
    let (node, remote) = sharing.join().expect("sharing");
    let (sender, received) = mpsc::channel();
    thread::spawn(move || consume(node, Some(remote), center, sender));
    let deadline = Instant::now() + WAIT;
    let frame = loop {
        if let Ok(frame) = received.try_recv() {
            break frame;
        }
        assert!(Instant::now() < deadline, "no frame reached the app");
        desktop.step();
    };
    let _ = frontend.kill();
    let _ = frontend.wait();

    let picked = picked.expect("the picker was shown");
    assert_eq!(picked.len(), 1, "{picked:?}");
    assert!(picked[0].starts_with("monitor:"), "{picked:?}");
    assert_eq!(
        (frame.width, frame.height),
        (OUTPUT.0 as u32, OUTPUT.1 as u32)
    );
    assert_eq!(&frame.pixel[..3], &[0x00, 0x00, 0xff]);
}
