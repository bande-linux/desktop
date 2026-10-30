use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};

use unbound_audio::Audio;
use unbound_sound::{Alert, BUS_NAME, OBJECT_PATH, Policy, Sound, write_theme};
use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;

#[zbus::proxy(
    interface = "org.unbound.Config",
    default_service = "org.unbound.Config",
    default_path = "/org/unbound/Config",
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

struct SoundService {
    audio: Mutex<Option<Audio>>,
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

    fn output(&self, sound: Sound, gain: f32) -> Result<(), unbound_audio::Error> {
        let mut audio = locked(&self.audio);
        if audio.is_none() {
            *audio = Some(Audio::connect()?);
        }
        let Some(connected) = audio.as_ref() else {
            return Ok(());
        };
        if connected
            .snapshot()
            .default_output()
            .is_none_or(|output| output.level.muted)
        {
            return Ok(());
        }
        let result = connected.play(self.samples(sound), gain);
        if result == Err(unbound_audio::Error::Disconnected) {
            *audio = None;
        }
        result
    }
}

#[zbus::interface(name = "org.unbound.Sound")]
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
    let service = SoundService {
        audio: Mutex::new(
            Audio::connect()
                .inspect_err(|error| tracing::warn!("no sound yet: {error}"))
                .ok(),
        ),
        samples: Mutex::default(),
        config,
    };
    Ok(zbus::blocking::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, service)?
        .build()?)
}

fn main() -> ExitCode {
    unbound_log::init("unbound-sound");
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
            unbound_bus::serve_until_closed(&connection);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
