#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{PipeReader, Read};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use unbound_power::logind::SleepGuard;
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

const WAIT: Duration = Duration::from_secs(5);
const SHORT_WAIT: Duration = Duration::from_millis(300);
const PROMPTLY: Duration = Duration::from_secs(1);
const SESSION_PATH: &str = "/org/freedesktop/login1/session/_31";
const MANAGER_PATH: &str = "/org/freedesktop/login1";

#[derive(Debug, PartialEq)]
enum Event {
    Inhibited(String, String),
    Locked,
    Released,
}

type Events = Arc<Mutex<Sender<Event>>>;

fn send(events: &Events, event: Event) {
    if let Ok(sender) = events.lock() {
        let _ = sender.send(event);
    }
}

fn watch_release(reader: PipeReader, events: Events) {
    thread::spawn(move || {
        let mut reader = reader;
        let mut byte = [0u8];
        if matches!(reader.read(&mut byte), Ok(0)) {
            send(&events, Event::Released);
        }
    });
}

struct FakeManager {
    events: Events,
}

#[zbus::interface(name = "org.freedesktop.login1.Manager")]
impl FakeManager {
    fn inhibit(
        &self,
        what: String,
        _who: String,
        _why: String,
        mode: String,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        let (reader, writer) =
            std::io::pipe().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        send(&self.events, Event::Inhibited(what, mode));
        watch_release(reader, Arc::clone(&self.events));
        Ok(OwnedFd::from(writer).into())
    }

    fn get_session(&self, session_id: String) -> zbus::fdo::Result<OwnedObjectPath> {
        if session_id != "auto" {
            return Err(zbus::fdo::Error::Failed(format!("no session {session_id}")));
        }
        OwnedObjectPath::try_from(SESSION_PATH)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    #[zbus(signal)]
    async fn prepare_for_sleep(emitter: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
}

struct FakeSession {
    events: Events,
    locked: Arc<AtomicBool>,
}

#[zbus::interface(name = "org.freedesktop.login1.Session")]
impl FakeSession {
    fn lock(&self) {
        send(&self.events, Event::Locked);
    }

    async fn set_locked_hint(
        &self,
        locked: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.locked.store(locked, Ordering::SeqCst);
        self.locked_hint_changed(&emitter).await?;
        Ok(())
    }

    #[zbus(property)]
    fn locked_hint(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }
}

struct Fixture {
    _guard: SleepGuard,
    logind: Connection,
    events: Receiver<Event>,
    _bus: SessionBus,
}

fn start(already_locked: bool, lock_wait: Duration) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let logind = bus.connect().expect("logind connection");
    let (sender, events) = mpsc::channel();
    let shared: Events = Arc::new(Mutex::new(sender));
    logind
        .object_server()
        .at(
            MANAGER_PATH,
            FakeManager {
                events: Arc::clone(&shared),
            },
        )
        .expect("manager");
    logind
        .object_server()
        .at(
            SESSION_PATH,
            FakeSession {
                events: shared,
                locked: Arc::new(AtomicBool::new(already_locked)),
            },
        )
        .expect("session");
    logind
        .request_name("org.freedesktop.login1")
        .expect("logind name");
    let guard = SleepGuard::start(&bus.connect().expect("guard connection"), lock_wait)
        .expect("sleep guard");
    Fixture {
        _guard: guard,
        logind,
        events,
        _bus: bus,
    }
}

impl Fixture {
    fn next(&self) -> Event {
        self.events.recv_timeout(WAIT).expect("event")
    }

    fn quiet_for(&self, time: Duration) -> bool {
        self.events.recv_timeout(time).is_err()
    }

    fn lock_screen_shown(&self) {
        self.logind
            .call_method(
                Some("org.freedesktop.login1"),
                SESSION_PATH,
                Some("org.freedesktop.login1.Session"),
                "SetLockedHint",
                &(true,),
            )
            .expect("SetLockedHint");
    }

    fn prepare_for_sleep(&self, start: bool) {
        let emitter = SignalEmitter::new(self.logind.inner(), MANAGER_PATH).expect("emitter");
        zbus::block_on(FakeManager::prepare_for_sleep(&emitter, start)).expect("signal");
    }
}

fn delay_sleep() -> Event {
    Event::Inhibited("sleep".to_owned(), "delay".to_owned())
}

#[test]
fn the_guard_delays_sleep_from_the_start() {
    let fixture = start(false, SHORT_WAIT);
    assert_eq!(fixture.next(), delay_sleep());
}

#[test]
fn sleep_waits_until_the_lock_screen_is_on_screen() {
    let fixture = start(false, WAIT);
    fixture.next();
    fixture.prepare_for_sleep(true);
    assert_eq!(fixture.next(), Event::Locked);
    assert!(fixture.quiet_for(SHORT_WAIT));

    fixture.lock_screen_shown();
    assert_eq!(fixture.events.recv_timeout(PROMPTLY), Ok(Event::Released));
}

#[test]
fn a_locked_session_releases_the_delay_at_once() {
    let fixture = start(true, WAIT);
    fixture.next();
    fixture.prepare_for_sleep(true);
    assert_eq!(fixture.events.recv_timeout(PROMPTLY), Ok(Event::Released));
}

#[test]
fn a_lock_screen_that_never_shows_does_not_hold_sleep_back() {
    let fixture = start(false, SHORT_WAIT);
    fixture.next();
    fixture.prepare_for_sleep(true);
    assert_eq!(fixture.next(), Event::Locked);
    assert_eq!(fixture.next(), Event::Released);
}

#[test]
fn waking_up_takes_a_new_delay() {
    let fixture = start(false, SHORT_WAIT);
    fixture.next();
    fixture.prepare_for_sleep(true);
    fixture.next();
    fixture.next();
    fixture.prepare_for_sleep(false);
    assert_eq!(fixture.next(), delay_sleep());
}
