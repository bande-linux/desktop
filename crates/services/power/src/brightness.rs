use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, poll};
use zbus::blocking::Connection;
use zbus::blocking::fdo::{PropertiesChangedIterator, PropertiesProxy};
use zbus::blocking::object_server::InterfaceRef;
use zbus::proxy::CacheProperties;

use crate::backlight::{Backlight, dimmed};
use crate::ddc::{I2cWire, Monitor, Reading, SYSFS_DRM, Wire, connector_bus};
use crate::logind::SessionProxyBlocking;

pub const BUS_NAME: &str = "org.bande.Brightness";
pub const OBJECT_PATH: &str = "/org/bande/Brightness";
const I2C_DEVICES: &str = "/dev";
const INTERNAL_CONNECTORS: [&str; 3] = ["eDP", "LVDS", "DSI"];
const LOWEST_SOFTWARE_GAIN: f64 = 0.2;
const IDLE_GAIN: f64 = 0.5;
const MONITOR_WRITE_GAP: Duration = Duration::from_millis(250);
const PROBE_WAITS: [Duration; 4] = [
    Duration::ZERO,
    Duration::from_secs(1),
    Duration::from_secs(3),
    Duration::from_secs(8),
];
const SAVE_SETTLE: Duration = Duration::from_secs(2);
const OWN_WRITE_ECHO: Duration = Duration::from_secs(1);
const READ_LIMIT: usize = 32;

type ConnectedDisplay = (String, String, String, bool);
pub type OpenWire = Arc<dyn Fn(&str) -> Option<Box<dyn Wire>> + Send + Sync>;

#[zbus::proxy(
    interface = "org.bande.Displays",
    default_service = "org.bande.Displays",
    default_path = "/org/bande/Displays"
)]
trait Displays {
    #[zbus(property)]
    fn connected(&self) -> zbus::Result<Vec<ConnectedDisplay>>;
    fn set_dim(&self, connector: &str, gain: f64) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "org.bande.Brightness",
    default_service = "org.bande.Brightness",
    default_path = "/org/bande/Brightness"
)]
pub trait Brightness {
    #[zbus(property)]
    fn displays(&self) -> zbus::Result<Vec<(String, String, f64)>>;
    fn set_level(&self, connector: &str, level: f64) -> zbus::Result<()>;
    fn refresh(&self) -> zbus::Result<()>;
}

fn software_gain(level: f64) -> f64 {
    LOWEST_SOFTWARE_GAIN + (1.0 - LOWEST_SOFTWARE_GAIN) * level.clamp(0.0, 1.0)
}

fn is_internal(connector: &str) -> bool {
    INTERNAL_CONNECTORS
        .iter()
        .any(|prefix| connector.starts_with(prefix))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Wanted {
    level: Option<f64>,
    known: Option<f64>,
    dimmed: Option<bool>,
    read: bool,
    reapply: bool,
    stop: bool,
}

#[derive(Default)]
struct Mailbox {
    wanted: Mutex<Wanted>,
    posted: Condvar,
}

impl Mailbox {
    fn post(&self, fill: impl FnOnce(&mut Wanted)) {
        fill(&mut lock(&self.wanted));
        self.posted.notify_one();
    }

    fn take(&self, until: Option<Instant>) -> Wanted {
        let mut wanted = lock(&self.wanted);
        let empty = |wanted: &Wanted| {
            wanted.level.is_none()
                && wanted.known.is_none()
                && wanted.dimmed.is_none()
                && !wanted.read
                && !wanted.reapply
                && !wanted.stop
        };
        wanted = match until {
            Some(until) => {
                let wait = until.saturating_duration_since(Instant::now());
                self.posted
                    .wait_timeout_while(wanted, wait, |wanted| empty(wanted))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => self
                .posted
                .wait_while(wanted, |wanted| empty(wanted))
                .unwrap_or_else(PoisonError::into_inner),
        };
        std::mem::take(&mut *wanted)
    }
}

struct Light {
    connector: String,
    identity: String,
    name: String,
    level: Option<f64>,
    mailbox: Arc<Mailbox>,
}

pub struct Options {
    pub backlights: PathBuf,
    pub saved: Option<PathBuf>,
    pub open_wire: OpenWire,
}

impl Options {
    pub fn system(backlights: &Path) -> Self {
        Self {
            backlights: backlights.to_owned(),
            saved: bande_xdg::state_home().map(|state| state.join("bande/brightness")),
            open_wire: Arc::new(open_i2c),
        }
    }
}

fn open_i2c(connector: &str) -> Option<Box<dyn Wire>> {
    let path = connector_bus(Path::new(SYSFS_DRM), Path::new(I2C_DEVICES), connector)?;
    match I2cWire::open(&path) {
        Ok(wire) => Some(Box::new(wire)),
        Err(error) => {
            tracing::info!("no DDC on {connector} through {}: {error}", path.display());
            None
        }
    }
}

struct Shared {
    lights: Mutex<Vec<Light>>,
    dimmed: Mutex<bool>,
    saved: Mutex<BTreeMap<String, f64>>,
    changed: Sender<()>,
    save: Sender<()>,
    backlight: Option<Backlight>,
    own_backlight_write: Mutex<Option<Instant>>,
    logind: Option<SessionProxyBlocking<'static>>,
    compositor: DisplaysProxyBlocking<'static>,
    open_wire: OpenWire,
}

impl Shared {
    fn listed(&self) -> Vec<(String, String, f64)> {
        let lights = lock(&self.lights);
        lights
            .iter()
            .filter_map(|light| {
                let level = light.level?;
                let twin = lights
                    .iter()
                    .filter(|other| other.level.is_some() && other.name == light.name)
                    .count()
                    > 1;
                let name = match twin {
                    true => format!("{} ({})", light.name, light.connector),
                    false => light.name.clone(),
                };
                Some((light.connector.clone(), name, level))
            })
            .collect()
    }

    fn report(&self, connector: &str, level: Option<f64>) {
        let mut lights = lock(&self.lights);
        if let Some(light) = lights.iter_mut().find(|light| light.connector == connector)
            && light.level != level
        {
            light.level = level;
            let _ = self.changed.send(());
        }
    }

    fn remember(&self, identity: &str, level: f64) {
        let previous = lock(&self.saved).insert(identity.to_owned(), level);
        if previous != Some(level) {
            let _ = self.save.send(());
        }
    }

    fn forget(&self, identity: &str) {
        if lock(&self.saved).remove(identity).is_some() {
            let _ = self.save.send(());
        }
    }

    fn set_level(&self, connector: &str, level: f64) {
        let level = level.clamp(0.0, 1.0);
        let mut lights = lock(&self.lights);
        let Some(light) = lights
            .iter_mut()
            .find(|light| light.connector == connector && light.level.is_some())
        else {
            return;
        };
        if light.level != Some(level) {
            light.level = Some(level);
            let _ = self.changed.send(());
        }
        light.mailbox.post(|wanted| wanted.level = Some(level));
    }

    fn sync(self: &Arc<Self>, heads: Vec<ConnectedDisplay>) {
        let mut lights = lock(&self.lights);
        lights.retain(|light| {
            let stays = heads
                .iter()
                .any(|head| head.0 == light.connector && head.1 == light.identity);
            light.mailbox.post(|wanted| match stays {
                true => wanted.reapply = true,
                false => wanted.stop = true,
            });
            stays
        });
        for (connector, identity, name, can_dim) in heads {
            if lights.iter().any(|light| light.connector == connector) {
                continue;
            }
            let mailbox = Arc::new(Mailbox::default());
            let lamp = Lamp::new(self, connector.clone(), identity.clone(), can_dim, &mailbox);
            let level = lamp.shown_level();
            let started = thread::Builder::new()
                .name("brightness".to_owned())
                .spawn(move || lamp.run());
            if let Err(error) = started {
                tracing::warn!("cannot control the brightness of {connector}: {error}");
                continue;
            }
            lights.push(Light {
                connector,
                identity,
                name,
                level,
                mailbox,
            });
        }
        lights.sort_by_key(|light| !is_internal(&light.connector));
        let _ = self.changed.send(());
    }

    fn backlight_moved(&self, level: f64) {
        if *lock(&self.dimmed)
            || lock(&self.own_backlight_write).is_some_and(|at| at.elapsed() < OWN_WRITE_ECHO)
        {
            return;
        }
        let mut lights = lock(&self.lights);
        if let Some(light) = lights
            .iter_mut()
            .find(|light| is_internal(&light.connector) && light.level.is_some())
            && light.level != Some(level)
        {
            light.level = Some(level);
            light.mailbox.post(|wanted| wanted.known = Some(level));
            let _ = self.changed.send(());
        }
    }
}

enum Drive {
    Backlight(Backlight),
    Monitor(Monitor, u16),
    Gain,
    Nothing,
}

struct Lamp {
    shared: Arc<Shared>,
    mailbox: Arc<Mailbox>,
    connector: String,
    identity: String,
    can_dim: bool,
    drive: Drive,
    level: f64,
    dimmed: bool,
    unapplied: bool,
    sent_gain: Option<f64>,
    sent_value: Option<u16>,
    wrote_at: Option<Instant>,
    probes: std::slice::Iter<'static, Duration>,
    next_probe: Option<Instant>,
}

fn backlight_level(backlight: &Backlight) -> Option<f64> {
    let maximum = backlight.max_brightness().ok().filter(|most| *most > 0)?;
    Some(f64::from(backlight.brightness().ok()?.min(maximum)) / f64::from(maximum))
}

impl Lamp {
    fn new(
        shared: &Arc<Shared>,
        connector: String,
        identity: String,
        can_dim: bool,
        mailbox: &Arc<Mailbox>,
    ) -> Self {
        let internal_backlight = shared
            .backlight
            .clone()
            .filter(|_| is_internal(&connector) && shared.logind.is_some())
            .and_then(|backlight| backlight_level(&backlight).map(|level| (backlight, level)));
        let (drive, level) = match internal_backlight {
            Some((backlight, level)) => (Drive::Backlight(backlight), level),
            None => (
                if can_dim { Drive::Gain } else { Drive::Nothing },
                lock(&shared.saved).get(&identity).copied().unwrap_or(1.0),
            ),
        };
        let unapplied = !matches!(drive, Drive::Backlight(_));
        let mut lamp = Self {
            shared: Arc::clone(shared),
            mailbox: Arc::clone(mailbox),
            connector,
            identity,
            can_dim,
            drive,
            level,
            dimmed: *lock(&shared.dimmed),
            unapplied,
            sent_gain: None,
            sent_value: None,
            wrote_at: None,
            probes: PROBE_WAITS.iter(),
            next_probe: None,
        };
        lamp.plan_probe();
        lamp
    }

    fn shown_level(&self) -> Option<f64> {
        (!matches!(self.drive, Drive::Nothing)).then_some(self.level)
    }

    fn plan_probe(&mut self) {
        self.next_probe = match self.drive {
            Drive::Backlight(_) | Drive::Monitor(..) => None,
            Drive::Gain | Drive::Nothing => self.probes.next().map(|wait| Instant::now() + *wait),
        };
    }

    fn run(mut self) {
        loop {
            let wanted = self.mailbox.take(self.wake_at());
            if wanted.stop {
                return;
            }
            if let Some(known) = wanted.known {
                self.level = known;
            }
            if let Some(level) = wanted.level {
                self.level = level;
                self.unapplied = true;
            }
            if let Some(dimmed) = wanted.dimmed {
                self.dimmed = dimmed;
                self.unapplied = true;
            }
            if wanted.reapply {
                self.sent_gain = None;
                self.unapplied = true;
            }
            if wanted.read {
                self.read();
            }
            if self.next_probe.is_some_and(|at| at <= Instant::now()) {
                self.probe();
            }
            if self.unapplied {
                self.apply();
            }
        }
    }

    fn wake_at(&self) -> Option<Instant> {
        let write = self.unapplied.then(|| match self.wrote_at {
            Some(at) if matches!(self.drive, Drive::Monitor(..)) => at + MONITOR_WRITE_GAP,
            _ => Instant::now(),
        });
        write.into_iter().chain(self.next_probe).min()
    }

    fn probe(&mut self) {
        let wire = (self.shared.open_wire)(&self.connector);
        let Some(wire) = wire else {
            self.next_probe = None;
            return;
        };
        let mut monitor = Monitor::new(wire);
        match monitor.brightness() {
            Ok(reading) => self.use_monitor(monitor, reading),
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                tracing::info!("{}: {error}", self.connector);
                self.next_probe = None;
            }
            Err(error) => {
                tracing::debug!("{} does not answer DDC yet: {error}", self.connector);
                self.plan_probe();
            }
        }
    }

    fn use_monitor(&mut self, monitor: Monitor, reading: Reading) {
        tracing::info!("{} takes its brightness over DDC", self.connector);
        self.level = f64::from(reading.value) / f64::from(reading.maximum);
        self.sent_value = Some(reading.value);
        self.drive = Drive::Monitor(monitor, reading.maximum);
        self.next_probe = None;
        self.unapplied = true;
        self.shared.forget(&self.identity);
        self.shared.report(&self.connector, Some(self.level));
    }

    fn read(&mut self) {
        match &mut self.drive {
            Drive::Monitor(monitor, _) if !self.unapplied => match monitor.brightness() {
                Ok(reading) => {
                    self.level = f64::from(reading.value) / f64::from(reading.maximum);
                    self.sent_value = Some(reading.value);
                    self.shared.report(&self.connector, Some(self.level));
                }
                Err(error) => tracing::debug!("cannot read {}: {error}", self.connector),
            },
            Drive::Gain | Drive::Nothing if self.next_probe.is_none() => self.probe(),
            _ => {}
        }
    }

    fn send_gain(&mut self, gain: f64) {
        if !self.can_dim || self.sent_gain == Some(gain) {
            return;
        }
        match self.shared.compositor.set_dim(&self.connector, gain) {
            Ok(()) => self.sent_gain = Some(gain),
            Err(error) => tracing::warn!("cannot dim {}: {error}", self.connector),
        }
    }

    fn apply(&mut self) {
        let idle = if self.dimmed { IDLE_GAIN } else { 1.0 };
        self.unapplied = false;
        match &mut self.drive {
            Drive::Backlight(backlight) => {
                let written = backlight.max_brightness().and_then(|maximum| {
                    let raw = ((self.level * f64::from(maximum)).round() as u32).clamp(1, maximum);
                    let raw = match self.dimmed {
                        true => dimmed(raw).unwrap_or(raw),
                        false => raw,
                    };
                    *lock(&self.shared.own_backlight_write) = Some(Instant::now());
                    match &self.shared.logind {
                        Some(logind) => logind
                            .set_brightness("backlight", &backlight.name, raw)
                            .map_err(io::Error::other),
                        None => Ok(()),
                    }
                });
                if let Err(error) = written {
                    tracing::warn!("cannot set the backlight: {error}");
                }
            }
            Drive::Monitor(monitor, maximum) => {
                let value = (self.level * f64::from(*maximum)).round() as u16;
                let rested = self
                    .wrote_at
                    .is_none_or(|at| at.elapsed() >= MONITOR_WRITE_GAP);
                if self.sent_value != Some(value) {
                    if rested {
                        self.wrote_at = Some(Instant::now());
                        match monitor.set_brightness(value) {
                            Ok(()) => self.sent_value = Some(value),
                            Err(error) => {
                                tracing::warn!("{} stopped answering DDC: {error}", self.connector);
                                self.lost_monitor();
                                return self.apply();
                            }
                        }
                    } else {
                        self.unapplied = true;
                    }
                }
                self.send_gain(idle);
            }
            Drive::Gain => {
                self.shared.remember(&self.identity, self.level);
                self.send_gain(software_gain(self.level) * idle);
            }
            Drive::Nothing => {}
        }
    }

    fn lost_monitor(&mut self) {
        self.sent_value = None;
        self.wrote_at = None;
        self.drive = if self.can_dim {
            Drive::Gain
        } else {
            Drive::Nothing
        };
        self.shared.report(&self.connector, self.shown_level());
        self.probes = PROBE_WAITS[1..].iter();
        self.plan_probe();
    }
}

fn saved_levels(path: Option<&Path>) -> BTreeMap<String, f64> {
    path.and_then(|path| fs::read_to_string(path).ok())
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    let (level, identity) = line.split_once('\t')?;
                    Some((identity.to_owned(), level.parse().ok()?))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn save_levels(shared: &Arc<Shared>, path: PathBuf, wanted: Receiver<()>) {
    while wanted.recv().is_ok() {
        thread::sleep(SAVE_SETTLE);
        wanted.try_iter().count();
        let text: String = lock(&shared.saved)
            .iter()
            .map(|(identity, level)| format!("{level}\t{identity}\n"))
            .collect();
        if let Err(error) = bande_xdg::replace_file(&path, text) {
            tracing::warn!("cannot remember the brightness: {error}");
        }
    }
}

fn follow_backlight(shared: Arc<Shared>, backlight: Backlight) -> io::Result<()> {
    let file = File::open(backlight.actual_brightness_file())?;
    let mut text = [0; READ_LIMIT];
    loop {
        let mut wanted = [PollFd::new(&file, PollFlags::PRI | PollFlags::ERR)];
        file.read_at(&mut text, 0)?;
        poll(&mut wanted, None)?;
        if let Some(level) = backlight_level(&backlight) {
            shared.backlight_moved(level);
        }
    }
}

struct Service {
    shared: Arc<Shared>,
}

#[zbus::interface(name = "org.bande.Brightness")]
impl Service {
    #[zbus(property)]
    fn displays(&self) -> Vec<(String, String, f64)> {
        self.shared.listed()
    }

    fn set_level(&self, connector: &str, level: f64) {
        self.shared.set_level(connector, level);
    }

    fn refresh(&self) {
        for light in lock(&self.shared.lights).iter() {
            light.mailbox.post(|wanted| wanted.read = true);
        }
    }
}

fn announce_changes(service: InterfaceRef<Service>, changed: Receiver<()>) {
    while changed.recv().is_ok() {
        changed.try_iter().count();
        let emitted = zbus::block_on(service.get().displays_changed(service.signal_emitter()));
        if let Err(error) = emitted {
            tracing::warn!("cannot announce the brightness: {error}");
        }
    }
}

fn follow_compositor(
    shared: Arc<Shared>,
    compositor: DisplaysProxyBlocking<'static>,
    changes: PropertiesChangedIterator,
) {
    if let Ok(heads) = compositor.connected() {
        shared.sync(heads);
    }
    for _ in changes {
        match compositor.connected() {
            Ok(heads) => shared.sync(heads),
            Err(error) => tracing::warn!("cannot read the connected displays: {error}"),
        }
    }
}

pub struct Lights {
    shared: Arc<Shared>,
}

impl Lights {
    pub fn start(
        session: &Connection,
        logind: Option<SessionProxyBlocking<'static>>,
        options: Options,
    ) -> zbus::Result<Self> {
        let compositor = DisplaysProxyBlocking::builder(session)
            .cache_properties(CacheProperties::No)
            .build()?;
        let (changed, changes) = mpsc::channel();
        let (save, saves) = mpsc::channel();
        let backlight = Backlight::find(&options.backlights);
        let shared = Arc::new(Shared {
            lights: Mutex::default(),
            dimmed: Mutex::new(false),
            saved: Mutex::new(saved_levels(options.saved.as_deref())),
            changed,
            save,
            backlight: backlight.clone(),
            own_backlight_write: Mutex::new(None),
            logind,
            compositor: compositor.clone(),
            open_wire: options.open_wire,
        });
        let server = session.object_server();
        server.at(
            OBJECT_PATH,
            Service {
                shared: Arc::clone(&shared),
            },
        )?;
        let service = server.interface::<_, Service>(OBJECT_PATH)?;
        session.request_name(BUS_NAME)?;
        thread::Builder::new()
            .name("brightness-announce".to_owned())
            .spawn(move || announce_changes(service, changes))?;
        if let Some(path) = options.saved {
            let saving = Arc::clone(&shared);
            thread::Builder::new()
                .name("brightness-save".to_owned())
                .spawn(move || save_levels(&saving, path, saves))?;
        }
        if let Some(backlight) = backlight {
            let following = Arc::clone(&shared);
            thread::Builder::new()
                .name("backlight-follow".to_owned())
                .spawn(move || {
                    if let Err(error) = follow_backlight(following, backlight) {
                        tracing::warn!("cannot follow the backlight: {error}");
                    }
                })?;
        }
        let changes = PropertiesProxy::builder(session)
            .destination(compositor.inner().destination().to_owned())?
            .path(compositor.inner().path().to_owned())?
            .build()?
            .receive_properties_changed()?;
        let following = Arc::clone(&shared);
        thread::Builder::new()
            .name("brightness-heads".to_owned())
            .spawn(move || follow_compositor(following, compositor, changes))?;
        Ok(Self { shared })
    }

    pub fn set_dimmed(&self, dimmed: bool) {
        *lock(&self.shared.dimmed) = dimmed;
        for light in lock(&self.shared.lights).iter() {
            light.mailbox.post(|wanted| wanted.dimmed = Some(dimmed));
        }
    }
}
