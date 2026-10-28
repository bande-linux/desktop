mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Options, Portal, Results, get};
use xdg_desktop_portal_unbound::OBJECT_PATH;
use xdg_desktop_portal_unbound::request::Cancellation;
use xdg_desktop_portal_unbound::screencast::{
    CURSOR_EMBEDDED, CURSOR_HIDDEN, CURSOR_METADATA, MONITOR, ScreenCast, ScreenCastBackend,
    Source, SourceRequest, Stream, VIRTUAL, WINDOW,
};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const INTERFACE: &str = "org.freedesktop.impl.portal.ScreenCast";
const SESSION: &str = "/org/freedesktop/portal/desktop/session/1_42/s";
const WAIT: Duration = Duration::from_secs(5);

struct FakeBackend {
    picks: AtomicUsize,
    stopped: Mutex<Sender<Vec<u32>>>,
}

fn monitor() -> Source {
    Source {
        kind: MONITOR,
        id: "monitor:eDP-1".to_owned(),
    }
}

impl ScreenCastBackend for FakeBackend {
    fn source_types(&self) -> u32 {
        MONITOR | WINDOW
    }

    fn cursor_modes(&self) -> u32 {
        CURSOR_HIDDEN | CURSOR_EMBEDDED
    }

    fn pick(&self, _request: &SourceRequest, _cancellation: &Cancellation) -> Option<Vec<Source>> {
        self.picks.fetch_add(1, Ordering::SeqCst);
        Some(vec![monitor()])
    }

    fn resolve(&self, ids: &[String]) -> Option<Vec<Source>> {
        (ids == [monitor().id]).then(|| vec![monitor()])
    }

    fn start(&self, sources: &[Source], _cursor_mode: u32) -> Result<Vec<Stream>, String> {
        Ok(sources
            .iter()
            .map(|source| Stream {
                node_id: 57,
                source: source.clone(),
                position: Some((0, 0)),
                size: Some((1920, 1080)),
            })
            .collect())
    }

    fn stop(&self, streams: &[Stream]) {
        if let Ok(stopped) = self.stopped.lock() {
            let _ = stopped.send(streams.iter().map(|stream| stream.node_id).collect());
        }
    }
}

struct Fixture {
    portal: Portal,
    backend: Arc<FakeBackend>,
    stopped: Receiver<Vec<u32>>,
}

fn start() -> Fixture {
    let (sender, stopped) = mpsc::channel();
    let backend = Arc::new(FakeBackend {
        picks: AtomicUsize::new(0),
        stopped: Mutex::new(sender),
    });
    let shared: Arc<dyn ScreenCastBackend> = backend.clone();
    let portal = Portal::start(|connection| {
        connection
            .object_server()
            .at(OBJECT_PATH, ScreenCast::new(shared))
            .expect("screencast");
    });
    Fixture {
        portal,
        backend,
        stopped,
    }
}

fn path(text: &str) -> ObjectPath<'_> {
    ObjectPath::try_from(text).expect("object path")
}

impl Fixture {
    fn create(&self, session: &str) -> u32 {
        self.portal
            .request(
                INTERFACE,
                "CreateSession",
                &(
                    path("/r/1"),
                    path(session),
                    "org.example.Meet",
                    Options::new(),
                ),
            )
            .0
    }

    fn select(&self, session: &str, options: Options<'_>) -> u32 {
        self.portal
            .request(
                INTERFACE,
                "SelectSources",
                &(path("/r/2"), path(session), "org.example.Meet", options),
            )
            .0
    }

    fn start(&self, session: &str) -> (u32, Results) {
        self.portal.request(
            INTERFACE,
            "Start",
            &(
                path("/r/3"),
                path(session),
                "org.example.Meet",
                "",
                Options::new(),
            ),
        )
    }
}

#[test]
fn a_session_streams_the_picked_monitor() {
    let fixture = start();
    assert_eq!(fixture.create(SESSION), 0);
    assert_eq!(fixture.select(SESSION, Options::new()), 0);
    let (response, results) = fixture.start(SESSION);
    assert_eq!(response, 0);

    let streams: Vec<(u32, std::collections::HashMap<String, OwnedValue>)> =
        get(&results, "streams");
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].0, 57);
    assert_eq!(
        u32::try_from(&streams[0].1["source_type"]).expect("u"),
        MONITOR
    );
    assert_eq!(
        <(i32, i32)>::try_from(streams[0].1["size"].try_clone().expect("clone")).expect("(ii)"),
        (1920, 1080)
    );
    assert_eq!(get::<u32>(&results, "persist_mode"), 0);
    assert!(!results.contains_key("restore_data"));
}

#[test]
fn starting_before_selecting_sources_fails() {
    let fixture = start();
    fixture.create(SESSION);
    assert_eq!(fixture.start(SESSION).0, 2);
}

#[test]
fn unavailable_source_types_and_cursor_modes_are_refused() {
    let fixture = start();
    fixture.create(SESSION);
    assert_eq!(
        fixture.select(SESSION, Options::from([("types", Value::from(VIRTUAL))])),
        2
    );
    assert_eq!(
        fixture.select(
            SESSION,
            Options::from([("cursor_mode", Value::from(CURSOR_METADATA))])
        ),
        2
    );
}

#[test]
fn a_persisted_selection_is_restored_without_asking_again() {
    let fixture = start();
    fixture.create(SESSION);
    fixture.select(
        SESSION,
        Options::from([("persist_mode", Value::from(2u32))]),
    );
    let (_, results) = fixture.start(SESSION);
    assert_eq!(get::<u32>(&results, "persist_mode"), 2);
    let restore_data = results["restore_data"].try_clone().expect("clone");
    assert_eq!(fixture.backend.picks.load(Ordering::SeqCst), 1);

    let second = "/org/freedesktop/portal/desktop/session/1_42/t";
    fixture.create(second);
    let restore = Value::from(restore_data);
    assert_eq!(
        fixture.select(
            second,
            Options::from([
                ("restore_data", restore),
                ("persist_mode", Value::from(2u32))
            ])
        ),
        0
    );
    assert_eq!(fixture.backend.picks.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.start(second).0, 0);
}

#[test]
fn closing_the_session_stops_its_streams() {
    let fixture = start();
    fixture.create(SESSION);
    fixture.select(SESSION, Options::new());
    fixture.start(SESSION);

    fixture
        .portal
        .close(SESSION, "org.freedesktop.impl.portal.Session");
    assert_eq!(fixture.stopped.recv_timeout(WAIT).expect("stopped"), [57]);
    assert_eq!(fixture.start(SESSION).0, 2);
}

#[test]
fn the_available_types_and_cursor_modes_are_the_backends() {
    let fixture = start();
    let get_property = |name: &str| -> u32 {
        let value: OwnedValue = fixture
            .portal
            .client
            .call_method(
                Some(xdg_desktop_portal_unbound::BUS_NAME),
                OBJECT_PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(INTERFACE, name),
            )
            .expect("Get")
            .body()
            .deserialize()
            .expect("v");
        u32::try_from(&value).expect("u")
    };
    assert_eq!(get_property("AvailableSourceTypes"), MONITOR | WINDOW);
    assert_eq!(
        get_property("AvailableCursorModes"),
        CURSOR_HIDDEN | CURSOR_EMBEDDED
    );
}
