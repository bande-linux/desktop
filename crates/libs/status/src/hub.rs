use std::mem::{Discriminant, discriminant};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_audio::Audio;
use bande_inbox::{Inbox, Outbox};
use rustix::event::{PollFd, PollFlags, poll};
use zbus::blocking::Connection;

use crate::Update;
use crate::clock::{follow_minutes, follow_zone};
use crate::devices::{Control, SoundControl, follow_devices, follow_sound};
use crate::tray::{Errand, host, run_errands};
use crate::unread::follow_unread;

#[derive(Default)]
struct Relay {
    subscribers: Vec<Outbox<Update>>,
    latest: Vec<(Discriminant<Update>, Update)>,
}

impl Relay {
    fn publish(&mut self, update: Update) {
        if remembered(&update) {
            let kind = discriminant(&update);
            self.latest.retain(|(known, _)| *known != kind);
            self.latest.push((kind, update.clone()));
        }
        self.subscribers
            .retain(|subscriber| subscriber.send(update.clone()));
    }

    fn subscribe(&mut self, outbox: Outbox<Update>) {
        if self
            .latest
            .iter()
            .all(|(_, update)| outbox.send(update.clone()))
        {
            self.subscribers.push(outbox);
        }
    }
}

fn remembered(update: &Update) -> bool {
    !matches!(
        update,
        Update::Menu { .. } | Update::AppMenu { .. } | Update::AppMenuChanged | Update::Minute
    )
}

#[derive(Clone)]
pub struct StatusHub {
    relay: Arc<Mutex<Relay>>,
    source: Outbox<Update>,
    devices: Option<Sender<Control>>,
    sound: Option<Sender<SoundControl>>,
    errands: Option<Sender<Errand>>,
    audio: Option<Arc<Audio>>,
    worker_lost: Arc<AtomicBool>,
}

fn lock(relay: &Mutex<Relay>) -> MutexGuard<'_, Relay> {
    relay.lock().unwrap_or_else(PoisonError::into_inner)
}

fn relay_updates(mut inbox: Inbox<Update>, relay: &Mutex<Relay>) {
    loop {
        let ready = inbox.fd();
        let mut fds = [PollFd::new(&ready, PollFlags::IN)];
        if let Err(error) = poll(&mut fds, None)
            && error != rustix::io::Errno::INTR
        {
            tracing::warn!("the bar and Control Center stop following the status: {error}");
            return;
        }
        let updates = inbox.drain();
        let mut relay = lock(relay);
        for update in updates {
            relay.publish(update);
        }
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(error) = thread::Builder::new().name(name.to_owned()).spawn(work) {
        tracing::warn!("cannot start {name}: {error}");
    }
}

impl StatusHub {
    pub fn start(
        session: Option<&Connection>,
        system: Option<&Connection>,
    ) -> std::io::Result<Self> {
        let inbox = Inbox::new()?;
        let source = inbox.outbox();
        let relay = Arc::new(Mutex::new(Relay::default()));
        let relaying = Arc::clone(&relay);
        spawn("status-relay", move || relay_updates(inbox, &relaying));
        let audio = Audio::connect()
            .inspect_err(|error| tracing::warn!("no sound: {error}"))
            .ok()
            .map(Arc::new);
        let sound = audio
            .as_ref()
            .map(|audio| follow_sound(Arc::clone(audio), source.clone()));
        let devices = system.map(|system| follow_devices(system, source.clone()));
        if let Some(system) = system
            && let Err(error) = follow_zone(system, source.clone())
        {
            tracing::warn!("the clock does not follow time zone changes: {error}");
        }
        let minutes = source.clone();
        spawn("status-clock", move || {
            if let Err(error) = follow_minutes(&minutes) {
                tracing::warn!("the clock does not advance: {error}");
            }
        });
        let errands = session.map(|session| {
            let (hosting, outbox) = (session.clone(), source.clone());
            spawn("status-tray", move || {
                if let Err(error) = host(&hosting, &outbox) {
                    tracing::warn!("cannot host status items: {error}");
                }
            });
            let (following, outbox) = (session.clone(), source.clone());
            spawn("status-unread", move || {
                if let Err(error) = follow_unread(&following, &outbox) {
                    tracing::info!("cannot follow unread notifications: {error}");
                }
            });
            let (errands, queue) = mpsc::channel();
            let (session, outbox) = (session.clone(), source.clone());
            spawn("status-errands", move || {
                run_errands(&session, &queue, &outbox);
            });
            errands
        });
        Ok(Self {
            relay,
            source,
            devices,
            sound,
            errands,
            audio,
            worker_lost: Arc::default(),
        })
    }

    pub fn subscribe(&self, outbox: Outbox<Update>) {
        lock(&self.relay).subscribe(outbox);
    }

    pub fn publisher(&self) -> Outbox<Update> {
        self.source.clone()
    }

    pub fn audio(&self) -> Option<Arc<Audio>> {
        self.audio.clone()
    }

    pub fn control(&self, control: Control) {
        self.deliver("battery, Wi-Fi and Bluetooth", &self.devices, control);
    }

    pub fn sound(&self, control: SoundControl) {
        self.deliver("sound", &self.sound, control);
    }

    pub fn errand(&self, errand: Errand) {
        self.deliver("status item", &self.errands, errand);
    }

    fn deliver<T>(&self, worker: &str, sender: &Option<Sender<T>>, message: T) {
        if let Some(sender) = sender
            && sender.send(message).is_err()
            && !self.worker_lost.swap(true, Ordering::Relaxed)
        {
            tracing::error!("the {worker} worker stopped; the bar no longer acts on it");
        }
    }
}
