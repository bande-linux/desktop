use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bande_audio::Audio;
use bande_sound::{Alert, BUS_NAME, OBJECT_PATH, Policy, Sound, write_theme};
use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;

#[zbus::proxy(
    interface = "org.bande.Config",
    default_service = "org.bande.Config",
    default_path = "/org/bande/Config",
    gen_async = false,
    blocking_name = "ConfigProxy"
)]
trait Config {
    #[zbus(property)]
    fn sound_effects(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn volume_feedback(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn alert_sound(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn alert_volume(&self) -> zbus::Result<f64>;
}

const CONNECT_PATIENCE: Duration = Duration::from_secs(2);

struct SoundService {
    audio: Audio,
    samples: Mutex<HashMap<Sound, Arc<[f32]>>>,
    config: Option<ConfigProxy<'static>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SoundService {
    fn policy(&self) -> Policy {
        let defaults = Policy::default();
        let Some(config) = &self.config else {
            return defaults;
        };
        Policy {
            effects: config.sound_effects().unwrap_or(defaults.effects),
            volume_feedback: config.volume_feedback().unwrap_or(defaults.volume_feedback),
            alert: config
                .alert_sound()
                .ok()
                .and_then(|keyword| Alert::from_keyword(&keyword))
                .unwrap_or(defaults.alert),
            alert_volume: config
                .alert_volume()
                .map_or(defaults.alert_volume, |volume| volume as f32),
        }
    }

    fn samples(&self, sound: Sound) -> Arc<[f32]> {
        Arc::clone(
            locked(&self.samples)
                .entry(sound)
                .or_insert_with(|| sound.samples().into()),
        )
    }

    fn output(&self, sound: Sound, gain: f32) -> Result<(), bande_audio::Error> {
        let audio = &self.audio;
        if !audio.wait_connected(CONNECT_PATIENCE) {
            return Err(bande_audio::Error::Disconnected);
        }
        if audio
            .snapshot()
            .default_output()
            .is_none_or(|output| output.level.muted)
        {
            return Ok(());
        }
        audio.play(self.samples(sound), gain)
    }
}

#[zbus::interface(name = "org.bande.Sound")]
impl SoundService {
    async fn play(&self, name: &str, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let policy = self.policy();
        let Some(sound) = Sound::from_name(name, policy.alert) else {
            tracing::debug!("no sound is called {name}");
            return;
        };
        if matches!(sound, Sound::Alert(_)) {
            let _ = Self::alert_played(&emitter).await;
        }
        let Some(gain) = policy.gain(sound) else {
            return;
        };
        if let Err(error) = self.output(sound, gain) {
            tracing::warn!("cannot play {name}: {error}");
        }
    }

    #[zbus(signal)]
    async fn alert_played(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

fn serve() -> Result<Connection, Box<dyn Error>> {
    let settings = Connection::session()?;
    let config = ConfigProxy::new(&settings)
        .inspect_err(|error| tracing::warn!("sound settings are not followed: {error}"))
        .ok();
    let audio = Audio::start()?;
    audio.follow_jacks();
    let service = SoundService {
        audio,
        samples: Mutex::default(),
        config,
    };
    Ok(zbus::blocking::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()?)
}

fn main() -> ExitCode {
    bande_log::init("bande-sound");
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some("--write-theme") {
        let directory = arguments
            .next()
            .map_or_else(|| PathBuf::from("."), PathBuf::from);
        return match write_theme(&directory) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                tracing::error!("cannot write the theme: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match serve() {
        Ok(connection) => {
            bande_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
