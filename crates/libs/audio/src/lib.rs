mod graph;
mod level;
mod stream;

use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use graph::{Command, Shared, State};
pub use level::{Level, VolumeKey, meter_level};
use pipewire as pw;
pub use stream::EFFECT_RATE;

const REPLY_CHECK: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Output,
    Input,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    Disconnected,
    NoSessionManager,
    NoSuchDevice,
    NoDefaultOutput,
    PipeWire(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => f.write_str("the connection to PipeWire was lost"),
            Self::NoSessionManager => f.write_str("no PipeWire session manager is running"),
            Self::NoSuchDevice => f.write_str("the sound device is gone"),
            Self::NoDefaultOutput => f.write_str("no sound output device is selected"),
            Self::PipeWire(message) => write!(f, "PipeWire: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<pw::Error> for Error {
    fn from(error: pw::Error) -> Self {
        Self::PipeWire(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub direction: Direction,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppStream {
    pub ids: Vec<u32>,
    pub name: String,
    pub icon: Option<String>,
    pub level: Level,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub generation: u64,
    pub connected: bool,
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    pub apps: Vec<AppStream>,
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

impl Snapshot {
    fn of(state: &State) -> Self {
        let mut devices: Vec<Device> = state
            .nodes
            .iter()
            .filter_map(|(id, node)| {
                let (channels, muted) = state.levels(node);
                Some(Device {
                    id: *id,
                    name: node.name.clone(),
                    description: node.description.clone(),
                    direction: node.direction?,
                    level: Level {
                        volume: level::from_channel(channels.iter().copied().fold(0.0, f32::max)),
                        muted,
                    },
                })
            })
            .filter(|device| !device.name.is_empty())
            .collect();
        devices.sort_by(|a, b| {
            a.description
                .to_lowercase()
                .cmp(&b.description.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        let (outputs, inputs) = devices
            .into_iter()
            .partition(|device| device.direction == Direction::Output);
        let mut streams: Vec<(&u32, &graph::NodeState)> = state
            .nodes
            .iter()
            .filter(|(_, node)| node.app.is_some() && !node.channels.is_empty())
            .collect();
        streams.sort_by_key(|(id, _)| **id);
        let mut apps: Vec<AppStream> = Vec::new();
        for (id, node) in streams {
            let Some(app) = &node.app else {
                continue;
            };
            let (channels, muted) = state.levels(node);
            let volume = level::from_channel(channels.iter().copied().fold(0.0, f32::max));
            match apps.iter_mut().find(|known| known.name == app.name) {
                Some(known) => {
                    known.ids.push(*id);
                    known.level.volume = known.level.volume.max(volume);
                    known.level.muted &= muted;
                }
                None => apps.push(AppStream {
                    ids: vec![*id],
                    name: app.name.clone(),
                    icon: app.icon.clone(),
                    level: Level { volume, muted },
                }),
            }
        }
        apps.sort_by_key(|app| app.name.to_lowercase());
        Self {
            generation: state.generation,
            connected: state.connected,
            outputs,
            inputs,
            apps,
            default_output: state.default_output.clone(),
            default_input: state.default_input.clone(),
        }
    }

    pub fn device(&self, name: &str) -> Option<&Device> {
        self.outputs
            .iter()
            .chain(&self.inputs)
            .find(|device| device.name == name)
    }

    pub fn default_output(&self) -> Option<&Device> {
        self.device(self.default_output.as_deref()?)
    }

    pub fn default_input(&self) -> Option<&Device> {
        self.device(self.default_input.as_deref()?)
    }
}

pub struct Audio {
    shared: Arc<Shared>,
    commands: pw::channel::Sender<Command>,
    thread: Option<JoinHandle<()>>,
}

pub struct Meter {
    id: u64,
    peak: Arc<AtomicU32>,
    commands: pw::channel::Sender<Command>,
}

impl Meter {
    pub fn take_peak(&self) -> f32 {
        f32::from_bits(self.peak.swap(0, Ordering::Relaxed)).min(1.0)
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop(self.id));
    }
}

static NEXT_METER: AtomicU64 = AtomicU64::new(1);

impl Audio {
    pub fn connect() -> Result<Self, Error> {
        let shared = Arc::new(Shared::default());
        let (commands, receiver) = pw::channel::channel();
        let (ready, started) = mpsc::channel();
        let graph = Arc::clone(&shared);
        let sender = commands.clone();
        let thread = std::thread::Builder::new()
            .name("pipewire".into())
            .spawn(move || graph::run(graph, receiver, sender, ready))
            .map_err(|error| Error::PipeWire(error.to_string()))?;
        let audio = Self {
            shared,
            commands,
            thread: Some(thread),
        };
        started.recv().map_err(|_| Error::Disconnected)??;
        Ok(audio)
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::of(&self.shared.lock())
    }

    pub fn wait_until(
        &self,
        timeout: Duration,
        ready: impl Fn(&Snapshot) -> bool,
    ) -> Option<Snapshot> {
        let deadline = Instant::now() + timeout;
        let mut state = self.shared.lock();
        loop {
            let snapshot = Snapshot::of(&state);
            if ready(&snapshot) {
                return Some(snapshot);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = self
                .shared
                .changed
                .wait_timeout(state, left)
                .map_or_else(|poisoned| poisoned.into_inner().0, |(state, _)| state);
        }
    }

    pub fn changed_since(&self, generation: Option<u64>) -> Snapshot {
        let mut state = self.shared.lock();
        while Some(state.generation) == generation {
            state = self
                .shared
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        Snapshot::of(&state)
    }

    fn send(&self, command: impl FnOnce(graph::Reply) -> Command) -> Result<(), Error> {
        let (done, outcome) = mpsc::channel();
        self.commands
            .send(command(done))
            .map_err(|_| Error::Disconnected)?;
        loop {
            match outcome.recv_timeout(REPLY_CHECK) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Timeout) if self.shared.lock().connected => {}
                Err(_) => return Err(Error::Disconnected),
            }
        }
    }

    pub fn set_default(&self, device: &Device) -> Result<(), Error> {
        self.send(|done| Command::SetDefault {
            direction: device.direction,
            name: device.name.clone(),
            done,
        })
    }

    pub fn set_level(&self, device: &Device, level: Level) -> Result<(), Error> {
        self.send(|done| Command::SetLevel {
            id: device.id,
            volume: Some(level.volume),
            muted: Some(level.muted),
            done,
        })
    }

    pub fn set_muted(&self, device: &Device, muted: bool) -> Result<(), Error> {
        self.send(|done| Command::SetLevel {
            id: device.id,
            volume: None,
            muted: Some(muted),
            done,
        })
    }

    pub fn set_app_level(&self, app: &AppStream, level: Level) -> Result<(), Error> {
        for id in &app.ids {
            self.send(|done| Command::SetLevel {
                id: *id,
                volume: Some(level.volume),
                muted: Some(level.muted),
                done,
            })?;
        }
        Ok(())
    }

    pub fn play(&self, samples: Arc<[f32]>, gain: f32) -> Result<(), Error> {
        self.send(|done| Command::Play {
            samples,
            gain,
            done,
        })
    }

    pub fn meter(&self, device: &Device) -> Result<Meter, Error> {
        let id = NEXT_METER.fetch_add(1, Ordering::Relaxed);
        let peak = Arc::new(AtomicU32::new(0));
        self.send(|done| Command::Meter {
            id,
            device: device.name.clone(),
            peak: Arc::clone(&peak),
            done,
        })?;
        Ok(Meter {
            id,
            peak,
            commands: self.commands.clone(),
        })
    }

    pub fn press(&self, key: VolumeKey, fine: bool) -> Result<Level, Error> {
        let snapshot = self.snapshot();
        let output = snapshot.default_output().ok_or(Error::NoDefaultOutput)?;
        let level = output.level.press(key, fine);
        self.set_level(output, level)?;
        Ok(level)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
