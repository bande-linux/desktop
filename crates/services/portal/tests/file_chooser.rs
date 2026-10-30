#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{Options, Portal, get};
use xdg_desktop_portal_unbound::OBJECT_PATH;
use xdg_desktop_portal_unbound::file_chooser::{
    Dialog, FileChooser, FileChooserUi, Filter, Mode, Outcome, Pattern, Selection,
};
use xdg_desktop_portal_unbound::request::Cancellation;
use zbus::zvariant::{ObjectPath, Value};

const INTERFACE: &str = "org.freedesktop.impl.portal.FileChooser";
const HANDLE: &str = "/org/freedesktop/portal/desktop/request/1_42/t";
const WAIT: Duration = Duration::from_secs(5);

struct FakeUi {
    shown: Mutex<Sender<Dialog>>,
    answer: Outcome,
    wait_for_cancel: bool,
    cancelled: Mutex<Sender<()>>,
}

impl FileChooserUi for FakeUi {
    fn choose(&self, dialog: &Dialog, cancellation: &Cancellation) -> Outcome {
        if let Ok(shown) = self.shown.lock() {
            let _ = shown.send(dialog.clone());
        }
        if self.wait_for_cancel {
            let (sender, closed) = mpsc::channel();
            cancellation.on_cancel(move || {
                let _ = sender.send(());
            });
            if closed.recv_timeout(WAIT).is_err() {
                return Outcome::Cancelled;
            }
            if let Ok(cancelled) = self.cancelled.lock() {
                let _ = cancelled.send(());
            }
            return Outcome::Cancelled;
        }
        self.answer.clone()
    }
}

struct Fixture {
    portal: Portal,
    shown: Receiver<Dialog>,
    cancelled: Receiver<()>,
}

fn start(answer: Outcome, wait_for_cancel: bool) -> Fixture {
    let (shown_sender, shown) = mpsc::channel();
    let (cancelled_sender, cancelled) = mpsc::channel();
    let ui = Arc::new(FakeUi {
        shown: Mutex::new(shown_sender),
        answer,
        wait_for_cancel,
        cancelled: Mutex::new(cancelled_sender),
    });
    let portal = Portal::start(|connection| {
        connection
            .object_server()
            .at(OBJECT_PATH, FileChooser::new(ui))
            .expect("file chooser");
    });
    Fixture {
        portal,
        shown,
        cancelled,
    }
}

fn handle() -> ObjectPath<'static> {
    ObjectPath::from_static_str_unchecked(HANDLE)
}

fn selected(paths: &[&str]) -> Selection {
    Selection {
        paths: paths.iter().map(PathBuf::from).collect(),
        ..Selection::default()
    }
}

#[test]
fn open_file_hands_the_options_to_the_dialog() {
    let fixture = start(Outcome::Chosen(selected(&["/home/anna/a.txt"])), false);
    let filters: Vec<(&str, Vec<(u32, &str)>)> = vec![
        ("Images", vec![(0, "*.png"), (1, "image/jpeg")]),
        ("Text", vec![(1, "text/plain")]),
    ];
    let options = Options::from([
        ("multiple", Value::from(true)),
        ("accept_label", Value::from("_Import")),
        ("filters", Value::from(filters)),
        (
            "current_folder",
            Value::from(b"/home/anna/Pictures\0".to_vec()),
        ),
    ]);
    let (response, _) = fixture.portal.request(
        INTERFACE,
        "OpenFile",
        &(
            handle(),
            "org.example.Viewer",
            "wayland:abc",
            "Open Images",
            options,
        ),
    );
    assert_eq!(response, 0);

    let dialog = fixture.shown.recv_timeout(WAIT).expect("dialog");
    assert_eq!(dialog.app_id, "org.example.Viewer");
    assert_eq!(dialog.title, "Open Images");
    assert_eq!(dialog.accept_label.as_deref(), Some("_Import"));
    assert!(dialog.modal, "modal defaults to true");
    assert_eq!(
        dialog.mode,
        Mode::Open {
            multiple: true,
            directory: false
        }
    );
    assert_eq!(
        dialog.filters[0],
        Filter {
            name: "Images".to_owned(),
            patterns: vec![
                Pattern::Glob("*.png".to_owned()),
                Pattern::MimeType("image/jpeg".to_owned())
            ],
        }
    );
    assert_eq!(dialog.folder, Some(PathBuf::from("/home/anna/Pictures")));
}

#[test]
fn chosen_paths_come_back_as_percent_encoded_file_uris() {
    let fixture = start(
        Outcome::Chosen(Selection {
            writable: true,
            ..selected(&["/home/anna/My Notes/caf\u{e9}#1.txt"])
        }),
        false,
    );
    let (response, results) = fixture.portal.request(
        INTERFACE,
        "OpenFile",
        &(handle(), "org.example.Editor", "", "Open", Options::new()),
    );
    assert_eq!(response, 0);
    assert_eq!(
        get::<Vec<String>>(&results, "uris"),
        ["file:///home/anna/My%20Notes/caf%C3%A9%231.txt"]
    );
    assert!(get::<bool>(&results, "writable"));
}

#[test]
fn save_files_appends_each_name_to_the_chosen_folder() {
    let fixture = start(Outcome::Chosen(selected(&["/home/anna/Export"])), false);
    let files = vec![b"one.png\0".to_vec(), b"two.png\0".to_vec()];
    let options = Options::from([("files", Value::from(files))]);
    let (response, results) = fixture.portal.request(
        INTERFACE,
        "SaveFiles",
        &(handle(), "org.example.Gallery", "", "Export", options),
    );
    assert_eq!(response, 0);
    assert_eq!(
        get::<Vec<String>>(&results, "uris"),
        [
            "file:///home/anna/Export/one.png",
            "file:///home/anna/Export/two.png"
        ]
    );
    assert!(!results.contains_key("writable"));
}

#[test]
fn save_file_offers_the_suggested_name() {
    let fixture = start(Outcome::Chosen(selected(&["/tmp/report.pdf"])), false);
    let options = Options::from([("current_name", Value::from("report.pdf"))]);
    fixture.portal.request(
        INTERFACE,
        "SaveFile",
        &(handle(), "org.example.Writer", "", "Save", options),
    );
    let dialog = fixture.shown.recv_timeout(WAIT).expect("dialog");
    assert_eq!(
        dialog.mode,
        Mode::Save {
            name: Some("report.pdf".to_owned()),
            file: None
        }
    );
}

#[test]
fn dismissing_the_dialog_answers_cancelled() {
    let fixture = start(Outcome::Cancelled, false);
    let (response, results) = fixture.portal.request(
        INTERFACE,
        "OpenFile",
        &(handle(), "org.example.Editor", "", "Open", Options::new()),
    );
    assert_eq!(response, 1);
    assert!(results.is_empty());
}

#[test]
fn closing_the_request_closes_the_dialog() {
    let fixture = start(Outcome::Cancelled, true);
    let client = fixture.portal.client.clone();
    let pending = std::thread::spawn(move || {
        client
            .call_method(
                Some(xdg_desktop_portal_unbound::BUS_NAME),
                OBJECT_PATH,
                Some(INTERFACE),
                "OpenFile",
                &(handle(), "org.example.Editor", "", "Open", Options::new()),
            )
            .expect("OpenFile")
            .body()
            .deserialize::<(u32, common::Results)>()
            .expect("reply")
    });
    fixture.shown.recv_timeout(WAIT).expect("dialog");

    fixture
        .portal
        .close(HANDLE, "org.freedesktop.impl.portal.Request");
    fixture.cancelled.recv_timeout(WAIT).expect("dialog closed");
    assert_eq!(pending.join().expect("thread").0, 1);
}

const FAKE_GTK: &str = "org.example.FakeGtkPortal";

struct FakeGtk {
    asked: Mutex<Sender<(String, String)>>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.FileChooser")]
impl FakeGtk {
    fn open_file(
        &self,
        handle: ObjectPath<'_>,
        app_id: String,
        _parent_window: String,
        _title: String,
        _options: HashMap<String, zbus::zvariant::OwnedValue>,
    ) -> (u32, HashMap<String, zbus::zvariant::OwnedValue>) {
        if let Ok(asked) = self.asked.lock() {
            let _ = asked.send((handle.to_string(), app_id));
        }
        let uris = zbus::zvariant::OwnedValue::try_from(Value::from(vec!["file:///srv/gtk.txt"]))
            .expect("uris");
        (0, HashMap::from([("uris".to_owned(), uris)]))
    }
}

#[test]
fn when_the_unbound_dialog_cannot_start_the_gtk_backend_answers_instead() {
    let (asked_sender, asked) = mpsc::channel();
    let (shown_sender, _shown) = mpsc::channel();
    let (cancelled_sender, _cancelled) = mpsc::channel();
    let ui = Arc::new(FakeUi {
        shown: Mutex::new(shown_sender),
        answer: Outcome::Unavailable,
        wait_for_cancel: false,
        cancelled: Mutex::new(cancelled_sender),
    });
    let portal = Portal::start(|connection| {
        connection
            .object_server()
            .at(
                OBJECT_PATH,
                FileChooser {
                    ui,
                    fallback: FAKE_GTK.to_owned(),
                },
            )
            .expect("file chooser");
    });
    let gtk = portal.bus.connect().expect("gtk connection");
    gtk.object_server()
        .at(
            OBJECT_PATH,
            FakeGtk {
                asked: Mutex::new(asked_sender),
            },
        )
        .expect("fake gtk");
    gtk.request_name(FAKE_GTK).expect("gtk name");

    let (response, results) = portal.request(
        INTERFACE,
        "OpenFile",
        &(handle(), "org.example.Editor", "", "Open", Options::new()),
    );
    assert_eq!(response, 0);
    assert_eq!(
        get::<Vec<String>>(&results, "uris"),
        ["file:///srv/gtk.txt"]
    );
    assert_eq!(
        asked.recv_timeout(WAIT).expect("forwarded"),
        (HANDLE.to_owned(), "org.example.Editor".to_owned())
    );
}
