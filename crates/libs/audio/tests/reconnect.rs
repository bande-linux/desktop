#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use bande_audio::Audio;

const PATIENCE: Duration = Duration::from_secs(20);
const CONFIG: &str = r#"
context.properties = { core.daemon = true core.name = pipewire-0 support.dbus = false }
context.spa-libs = { support.* = support/libspa-support }
context.modules = [
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-access }
]
"#;

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_daemon(directory: &Path) -> Daemon {
    let config = directory.join("pipewire.conf");
    fs::write(&config, CONFIG).unwrap();
    Daemon(
        Command::new("pipewire")
            .arg("-c")
            .arg(&config)
            .env("XDG_RUNTIME_DIR", directory)
            .env("PIPEWIRE_RUNTIME_DIR", directory)
            .spawn()
            .expect("the pipewire daemon"),
    )
}

fn wait_for(audio: &Audio, connected: bool) {
    let deadline = Instant::now() + PATIENCE;
    while audio.snapshot().connected != connected {
        assert!(
            Instant::now() < deadline,
            "connected never became {connected}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn sound_comes_back_after_pipewire_restarts_or_starts_late() {
    if bande_xdg::find_executable("pipewire").is_none() {
        eprintln!("skipped: no pipewire daemon here");
        return;
    }
    let directory = Path::new("/tmp").join(format!("bande-audio-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", &directory);
        std::env::set_var("PIPEWIRE_RUNTIME_DIR", &directory);
        std::env::remove_var("PIPEWIRE_REMOTE");
    }

    let audio = Audio::start().unwrap();
    assert!(!audio.wait_connected(Duration::from_secs(3)));

    let daemon = start_daemon(&directory);
    wait_for(&audio, true);
    let first = audio.snapshot().connection;

    drop(daemon);
    wait_for(&audio, false);

    let _daemon = start_daemon(&directory);
    wait_for(&audio, true);
    assert!(audio.snapshot().connection > first);

    drop(audio);
    let _ = fs::remove_dir_all(&directory);
}
