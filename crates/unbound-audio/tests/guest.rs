use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use unbound_audio::{Audio, Device, Level, Snapshot, VolumeKey};
use unbound_test_bus::SessionBus;

const WAIT: Duration = Duration::from_secs(10);

const DEVICES: &str = r#"
context.objects = [
    { factory = adapter args = { factory.name = support.null-audio-sink node.name = "speakers"
        node.description = "MacBook Pro Speakers" media.class = "Audio/Sink" audio.position = [ FL FR ]
        priority.session = 2000 object.linger = true } }
    { factory = adapter args = { factory.name = support.null-audio-sink node.name = "headphones"
        node.description = "AirPods" media.class = "Audio/Sink" audio.position = [ FL FR ]
        priority.session = 1000 object.linger = true } }
    { factory = adapter args = { factory.name = support.null-audio-sink node.name = "microphone"
        node.description = "MacBook Pro Microphone" media.class = "Audio/Source/Virtual" audio.position = [ MONO ]
        priority.session = 3000 object.linger = true } }
]
"#;

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

struct Session {
    audio: Option<Audio>,
    daemons: Vec<Child>,
    _bus: SessionBus,
    _home: TempDir,
}

impl Session {
    fn start() -> Self {
        let home = TempDir::new().expect("home");
        let runtime = home.path().join("run");
        let config = home.path().join("config");
        std::fs::create_dir_all(&runtime).expect("runtime dir");
        std::fs::create_dir_all(config.join("pipewire/pipewire.conf.d")).expect("config dir");
        std::fs::write(
            config.join("pipewire/pipewire.conf.d/devices.conf"),
            DEVICES,
        )
        .expect("devices");
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
        let audio = Audio::connect().expect("connect to PipeWire");
        let session = Self {
            audio: Some(audio),
            daemons,
            _bus: bus,
            _home: home,
        };
        session.until(|snapshot| {
            snapshot.outputs.len() == 2
                && snapshot.inputs.len() == 1
                && snapshot.default_output().is_some()
        });
        session
    }

    fn audio(&self) -> &Audio {
        self.audio.as_ref().expect("audio")
    }

    fn until(&self, ready: impl Fn(&Snapshot) -> bool) -> Snapshot {
        self.audio()
            .wait_until(WAIT, ready)
            .unwrap_or_else(|| panic!("PipeWire never got there: {:?}", self.audio().snapshot()))
    }

    fn device(&self, name: &str) -> Device {
        self.audio()
            .snapshot()
            .device(name)
            .cloned()
            .expect("device")
    }

    fn level_of(&self, name: &str, level: Level) {
        self.until(|snapshot| {
            snapshot.device(name).is_some_and(|device| {
                (device.level.volume - level.volume).abs() < 1e-3
                    && device.level.muted == level.muted
            })
        });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.audio = None;
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
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn level(volume: f32, muted: bool) -> Level {
    Level { volume, muted }
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn outputs_and_inputs_are_listed_by_name() {
    let session = Session::start();
    let snapshot = session.audio().snapshot();
    let names = |devices: &[Device]| -> Vec<String> {
        devices
            .iter()
            .map(|device| device.description.clone())
            .collect()
    };
    assert_eq!(
        names(&snapshot.outputs),
        ["AirPods", "MacBook Pro Speakers"]
    );
    assert_eq!(names(&snapshot.inputs), ["MacBook Pro Microphone"]);
    assert_eq!(snapshot.default_output.as_deref(), Some("speakers"));
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn choosing_an_output_makes_it_the_default() {
    let session = Session::start();
    session
        .audio()
        .set_default(&session.device("headphones"))
        .expect("choose");
    session.until(|snapshot| snapshot.default_output.as_deref() == Some("headphones"));
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn each_output_keeps_its_own_volume() {
    let session = Session::start();
    let audio = session.audio();
    audio
        .set_level(&session.device("speakers"), level(0.25, false))
        .expect("speakers");
    audio
        .set_level(&session.device("headphones"), level(0.75, false))
        .expect("headphones");
    session.level_of("speakers", level(0.25, false));
    session.level_of("headphones", level(0.75, false));

    audio
        .set_default(&session.device("headphones"))
        .expect("choose");
    session.until(|snapshot| snapshot.default_output.as_deref() == Some("headphones"));
    assert_eq!(
        audio.press(VolumeKey::Down, false).expect("down"),
        level(11.0 / 16.0, false)
    );
    session.level_of("headphones", level(11.0 / 16.0, false));
    session.level_of("speakers", level(0.25, false));
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn the_mute_key_silences_the_default_output_and_a_volume_key_brings_it_back() {
    let session = Session::start();
    let audio = session.audio();
    audio
        .set_level(&session.device("speakers"), level(0.5, false))
        .expect("speakers");
    session.level_of("speakers", level(0.5, false));

    audio.press(VolumeKey::Mute, false).expect("mute");
    session.level_of("speakers", level(0.5, true));
    audio.press(VolumeKey::Up, true).expect("up");
    session.level_of("speakers", level(33.0 / 64.0, false));
}
