use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::message::Header;
use zbus::names::{OwnedUniqueName, UniqueName};

use crate::state::{Headless, State};

pub const KEYBOARD_MONITOR_NAME: &str = "org.freedesktop.a11y.Manager";
const PATH: &str = "/org/freedesktop/a11y/Manager";
const INTERFACE: &str = "org.freedesktop.a11y.KeyboardMonitor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyEvent {
    pub(crate) released: bool,
    pub(crate) state: u32,
    pub(crate) keysym: u32,
    pub(crate) unichar: u32,
    pub(crate) keycode: u16,
}

struct Listener {
    name: OwnedUniqueName,
    watching: bool,
    grabbing: bool,
    modifiers: Vec<u32>,
    keystrokes: Vec<(u32, u32)>,
}

#[derive(Default)]
struct Listeners(Vec<Listener>);

impl Listeners {
    fn of(&mut self, name: &UniqueName<'_>) -> &mut Listener {
        let index = match self.0.iter().position(|listener| listener.name == *name) {
            Some(index) => index,
            None => {
                self.0.push(Listener {
                    name: name.to_owned().into(),
                    watching: false,
                    grabbing: false,
                    modifiers: Vec::new(),
                    keystrokes: Vec::new(),
                });
                self.0.len() - 1
            }
        };
        &mut self.0[index]
    }

    fn forget(&mut self, name: &str) {
        self.0.retain(|listener| listener.name.as_str() != name);
    }

    fn listening(&self) -> bool {
        self.0.iter().any(|listener| {
            listener.watching || listener.grabbing || !listener.modifiers.is_empty()
        })
    }

    fn grabbed(&self) -> bool {
        self.0.iter().any(|listener| listener.grabbing)
    }

    fn is_modifier(&self, keysym: u32) -> bool {
        self.0
            .iter()
            .any(|listener| listener.modifiers.contains(&keysym))
    }

    fn grabs(&self, keysym: u32, state: u32) -> bool {
        self.0.iter().any(|listener| {
            listener
                .keystrokes
                .iter()
                .any(|(grabbed, modifiers)| *grabbed == keysym && *modifiers == state)
        })
    }
}

fn lock(listeners: &Mutex<Listeners>) -> MutexGuard<'_, Listeners> {
    listeners.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Monitor {
    listeners: Arc<Mutex<Listeners>>,
}

fn sender<'a>(header: &'a Header<'_>) -> zbus::fdo::Result<&'a UniqueName<'a>> {
    header
        .sender()
        .ok_or_else(|| zbus::fdo::Error::AccessDenied("no sender".to_owned()))
}

#[zbus::interface(name = "org.freedesktop.a11y.KeyboardMonitor")]
impl Monitor {
    fn grab_keyboard(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        lock(&self.listeners).of(sender(&header)?).grabbing = true;
        Ok(())
    }

    fn ungrab_keyboard(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        lock(&self.listeners).of(sender(&header)?).grabbing = false;
        Ok(())
    }

    fn watch_keyboard(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        lock(&self.listeners).of(sender(&header)?).watching = true;
        Ok(())
    }

    fn unwatch_keyboard(&self, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        lock(&self.listeners).of(sender(&header)?).watching = false;
        Ok(())
    }

    fn set_key_grabs(
        &self,
        modifiers: Vec<u32>,
        keystrokes: Vec<(u32, u32)>,
        #[zbus(header)] header: Header<'_>,
    ) -> zbus::fdo::Result<()> {
        let mut listeners = lock(&self.listeners);
        let listener = listeners.of(sender(&header)?);
        listener.modifiers = modifiers;
        listener.keystrokes = keystrokes;
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct KeyboardMonitor {
    listeners: Arc<Mutex<Listeners>>,
    events: Option<Sender<KeyEvent>>,
    held_modifier: Option<u32>,
    consumed: Vec<u16>,
}

impl KeyboardMonitor {
    pub(crate) fn consumes(&mut self, event: KeyEvent) -> bool {
        let listeners = lock(&self.listeners);
        if !listeners.listening() {
            self.held_modifier = None;
            self.consumed.clear();
            return false;
        }
        let consumed = if event.released {
            let index = self.consumed.iter().position(|code| *code == event.keycode);
            if self.held_modifier == Some(event.keysym) {
                self.held_modifier = None;
            }
            index.map(|index| self.consumed.remove(index)).is_some()
        } else if listeners.is_modifier(event.keysym) {
            self.held_modifier = Some(event.keysym);
            true
        } else {
            listeners.grabbed()
                || self.held_modifier.is_some()
                || listeners.grabs(event.keysym, event.state)
        };
        drop(listeners);
        if consumed && !event.released {
            self.consumed.push(event.keycode);
        }
        if let Some(events) = &self.events {
            let _ = events.send(event);
        }
        consumed
    }
}

pub fn serve_keyboard_monitor(connection: &Connection) -> zbus::Result<KeyboardMonitorHandle> {
    let listeners = Arc::new(Mutex::new(Listeners::default()));
    connection.object_server().at(
        PATH,
        Monitor {
            listeners: Arc::clone(&listeners),
        },
    )?;
    connection.request_name(KEYBOARD_MONITOR_NAME)?;
    let departures = DBusProxy::new(connection)?.receive_name_owner_changed()?;
    let forgetting = Arc::clone(&listeners);
    thread::spawn(move || {
        for change in departures {
            if let Ok(args) = change.args()
                && args.new_owner().is_none()
            {
                lock(&forgetting).forget(args.name().as_str());
            }
        }
    });
    let (events, received) = channel::<KeyEvent>();
    let emitting = connection.clone();
    thread::spawn(move || {
        for event in received {
            let emitted = emitting.emit_signal(
                None::<()>,
                PATH,
                INTERFACE,
                "KeyEvent",
                &(
                    event.released,
                    event.state,
                    event.keysym,
                    event.unichar,
                    event.keycode,
                ),
            );
            if let Err(error) = emitted {
                tracing::debug!("cannot tell assistive technologies about a key: {error}");
            }
        }
    });
    Ok(KeyboardMonitorHandle { listeners, events })
}

pub struct KeyboardMonitorHandle {
    listeners: Arc<Mutex<Listeners>>,
    events: Sender<KeyEvent>,
}

impl State {
    pub(crate) fn monitor_key(&mut self, event: KeyEvent) -> bool {
        self.keyboard_monitor.consumes(event)
    }
}

impl Headless {
    pub fn monitor_keyboard(&mut self, handle: KeyboardMonitorHandle) {
        self.state.keyboard_monitor = KeyboardMonitor {
            listeners: handle.listeners,
            events: Some(handle.events),
            held_modifier: None,
            consumed: Vec::new(),
        };
    }
}
