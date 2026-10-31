#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use std::sync::Arc;
use tempfile::TempDir;

use bande_audio::{Audio, Device, EFFECT_RATE, Level, Snapshot, VolumeKey};
use bande_test_bus::SessionBus;

const WAIT: Duration = Duration::from_secs(10);

const DEVICES: &str = r#"
context.objects = [
    { factory = adapter args = { factory.name = support.null-audio-sink node.name = "speakers"
        node.description = "MacBook Pro Speakers" media.class = "Audio/Sink" audio.position = [ FL FR ]
        priority.session = 2000 node.always-process = true object.linger = true } }
    { factory = adapter args = { factory.name = support.null-audio-sink node.name = "headphones"
        node.description = "AirPods" media.class = "Audio/Sink" audio.position = [ FL FR ]
        priority.session = 1000 node.always-process = true object.linger = true } }
    { factory = adapter args = { factory.name = support.null-audio-sink node.name = "microphone"
        node.description = "MacBook Pro Microphone" media.class = "Audio/Source/Virtual" audio.position = [ MONO ]
        priority.session = 3000 node.always-process = true object.linger = true } }
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
        let deadline = Instant::now() + WAIT;
        loop {
            let snapshot = self.audio().snapshot();
            if ready(&snapshot) {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "PipeWire never got there: {snapshot:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
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
fn a_change_wakes_whoever_waits_for_one() {
    let session = Session::start();
    let audio = session.audio();
    let before = audio.snapshot();
    let speakers = session.device("speakers");
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| {
            let mut snapshot = audio.changed_since(Some(before.generation));
            while !snapshot
                .device("speakers")
                .is_some_and(|device| device.level.muted)
            {
                snapshot = audio.changed_since(Some(snapshot.generation));
            }
            snapshot
        });
        audio.set_muted(&speakers, true).expect("mute");
        assert!(waiter.join().expect("waiter").generation > before.generation);
    });
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

fn nodes_named(name: &str) -> usize {
    let dump = Command::new("pw-dump").output().expect("pw-dump");
    String::from_utf8_lossy(&dump.stdout)
        .matches(&format!("\"node.name\": \"{name}\""))
        .count()
}

fn eventually(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !ready() {
        assert!(Instant::now() < deadline, "never: {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn silence_wav(path: &Path, seconds: u32) {
    let samples = seconds * 8000;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&8000_u32.to_le_bytes());
    wav.extend_from_slice(&16000_u32.to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    wav.resize(wav.len() + (samples * 2) as usize, 0);
    std::fs::write(path, wav).expect("wav");
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn an_app_playing_sound_is_listed_by_name_and_its_volume_can_be_set() {
    let session = Session::start();
    let file = session._home.path().join("silence.wav");
    silence_wav(&file, 30);
    let mut player = Command::new("pw-play")
        .args([
            "-P",
            "{ application.name = \"Radio\" application.icon-name = \"radio\" }",
        ])
        .arg(&file)
        .spawn()
        .expect("pw-play");

    let snapshot = session.until(|snapshot| snapshot.apps.iter().any(|app| app.name == "Radio"));
    let radio = snapshot
        .apps
        .iter()
        .find(|app| app.name == "Radio")
        .cloned()
        .expect("radio");
    assert_eq!(radio.icon.as_deref(), Some("radio"));
    session
        .audio()
        .set_app_level(&radio, level(0.5, true))
        .expect("app level");
    session.until(|snapshot| {
        snapshot.apps.iter().any(|app| {
            app.name == "Radio" && (app.level.volume - 0.5).abs() < 1e-3 && app.level.muted
        })
    });

    let _ = player.kill();
    let _ = player.wait();
    session.until(|snapshot| snapshot.apps.is_empty());
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn a_sound_effect_plays_without_being_listed_as_an_app_and_then_goes_away() {
    let session = Session::start();
    let samples: Arc<[f32]> = vec![0.0; EFFECT_RATE as usize * 2].into();
    session.audio().play(samples, 0.5).expect("play");
    eventually("the effect plays", || {
        nodes_named("bande-sound-effect") == 1
    });
    assert!(session.audio().snapshot().apps.is_empty());
    eventually("the effect ends", || nodes_named("bande-sound-effect") == 0);
}

#[test]
#[ignore = "needs pipewire and wireplumber; ci/vm runs it in the guest"]
fn a_level_meter_listens_while_it_is_kept() {
    let session = Session::start();
    let meter = session
        .audio()
        .meter(&session.device("microphone"))
        .expect("meter");
    eventually("the meter listens", || {
        nodes_named("bande-level-meter") == 1
    });
    assert_eq!(meter.take_peak(), 0.0);
    drop(meter);
    eventually("the meter stops", || nodes_named("bande-level-meter") == 0);
}
