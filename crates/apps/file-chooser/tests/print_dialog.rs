#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixListener;
use std::process::Command;
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use bande_graphics::{Color, Item, Paint, Rect, Shape};
use bande_pdf::{Page, PageSetup, write};
use bande_printers::{BUS_NAME, Capabilities, JobInfo, JobOptions, Kind, OBJECT_PATH, PrinterInfo};
use bande_test_bus::SessionBus;
use bande_test_wayland::keys::KEY_ENTER;
use bande_test_wayland::{Frames, chord, headless};
use zbus::zvariant;

const PATIENCE: Duration = Duration::from_secs(30);

struct PrintService {
    submitted: Mutex<Sender<Vec<u8>>>,
}

#[zbus::interface(name = "org.bande.Print")]
impl PrintService {
    fn watch(&self) {}

    fn printers(&self) -> Vec<PrinterInfo> {
        vec![PrinterInfo {
            id: "office".to_owned(),
            name: "Office".to_owned(),
            model: String::new(),
            location: String::new(),
            kind: Kind::Direct,
            default: true,
            reachable: true,
        }]
    }

    fn capabilities(&self, _printer: String) -> Capabilities {
        Capabilities {
            media: vec!["iso_a4_210x297mm".to_owned()],
            default_media: "iso_a4_210x297mm".to_owned(),
            two_sided: false,
            color: true,
            maximum_copies: 99,
        }
    }

    fn jobs(&self) -> Vec<JobInfo> {
        Vec::new()
    }

    fn submit(
        &self,
        _title: String,
        _printer: String,
        _options: JobOptions,
        document: zvariant::OwnedFd,
    ) -> u32 {
        let mut bytes = Vec::new();
        File::from(OwnedFd::from(document))
            .read_to_end(&mut bytes)
            .unwrap();
        self.submitted.lock().unwrap().send(bytes).unwrap();
        1
    }

    #[zbus(signal)]
    async fn changed(emitter: &zbus::object_server::SignalEmitter<'_>) -> zbus::Result<()>;
}

#[test]
fn pressing_return_in_the_print_dialog_hands_the_document_to_the_print_service() {
    let bus = SessionBus::start().unwrap();
    let (sender, submitted) = channel();
    let service = bus.connect().unwrap();
    service
        .object_server()
        .at(
            OBJECT_PATH,
            PrintService {
                submitted: Mutex::new(sender),
            },
        )
        .unwrap();
    service.request_name(BUS_NAME).unwrap();

    let home = std::env::temp_dir().join(format!("bande-print-dialog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let setup = PageSetup::default();
    let mut page = Page::new(&setup);
    page.draw(Item::Fill {
        shape: Shape::rect(Rect::new(54.0, 54.0, 200.0, 80.0)),
        paint: Paint::Solid(Color::BLACK),
    });
    let document = home.join("document.pdf");
    std::fs::write(&document, write("Test", &[page]).unwrap()).unwrap();

    let socket = home.join("wayland");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let mut dialog = Command::new(env!("CARGO_BIN_EXE_bande-file-chooser"))
        .args(["--print", "--app-id", "org.example.App", "--title", "Test"])
        .arg("--document")
        .arg(&document)
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .env("WAYLAND_DISPLAY", &socket)
        .env("XDG_RUNTIME_DIR", &home)
        .spawn()
        .unwrap();

    let mut server = headless(1200, 900);
    let mut frames = Frames::start();
    let deadline = Instant::now() + PATIENCE;
    let body = loop {
        if let Ok(body) = submitted.try_recv() {
            break body;
        }
        if let Some(status) = dialog.try_wait().unwrap() {
            panic!("the dialog ended with {status} before handing over the document");
        }
        if Instant::now() > deadline {
            let _ = dialog.kill();
            panic!("the document was not handed over in {PATIENCE:?}");
        }
        if let Ok((stream, _)) = listener.accept() {
            stream.set_nonblocking(false).unwrap();
            server.insert_client(stream).unwrap();
        }
        let until = Instant::now() + Duration::from_millis(200);
        frames.run_until(&mut server, || Instant::now() > until);
        chord(&mut server, &[KEY_ENTER]);
    };
    assert!(body.starts_with(b"%PDF-"));
    let _ = dialog.kill();
    let _ = dialog.wait();
    let _ = std::fs::remove_dir_all(&home);
}
