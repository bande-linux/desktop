#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use bande_content::pdf::session::{Change, Session};
use bande_graphics::{Color, Item, Paint, Point, Rect, Shape, TextAlign, TextStyle, TextSystem};
use bande_pdf::{Page, PageSetup, write};
use bande_printers::ipp::{ENUM, INTEGER, JOB, KEYWORD, MIME_TYPE, NAME, PRINTER, Request};
use bande_printers::{JobOptions, PrintClient};
use bande_test_bus::SessionBus;

const PRINT_JOB: u16 = 0x0002;
const GET_JOB: u16 = 0x0009;
const COMPLETED: i32 = 9;

fn read_request(stream: &mut BufReader<TcpStream>) -> Option<Vec<u8>> {
    let mut line = String::new();
    let (mut length, mut chunked) = (0, false);
    stream.read_line(&mut line).ok()?;
    line.clear();
    while stream.read_line(&mut line).ok()? > 0 {
        let header = line.trim_end().to_lowercase();
        if header.is_empty() {
            break;
        }
        if let Some(value) = header.strip_prefix("content-length: ") {
            length = value.parse().unwrap();
        }
        chunked |= header == "transfer-encoding: chunked";
        line.clear();
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            stream.read_line(&mut line).ok()?;
            let size = usize::from_str_radix(line.trim(), 16).ok()?;
            let start = body.len();
            body.resize(start + size, 0);
            stream.read_exact(&mut body[start..]).ok()?;
            line.clear();
            stream.read_line(&mut line).ok()?;
            if size == 0 {
                break;
            }
        }
    } else {
        body.resize(length, 0);
        stream.read_exact(&mut body).ok()?;
    }
    Some(body)
}

fn answer(operation: u16, formats: &[&str]) -> Vec<u8> {
    let mut reply = Request::new(0, "printer");
    match operation {
        PRINT_JOB => {
            reply.begin_group(JOB);
            reply.integer(INTEGER, "job-id", 7);
        }
        GET_JOB => {
            reply.begin_group(JOB);
            reply.integer(ENUM, "job-state", COMPLETED);
        }
        _ => {
            reply.begin_group(PRINTER);
            reply
                .string(NAME, "printer-make-and-model", "Acme Test")
                .strings(MIME_TYPE, "document-format-supported", formats)
                .strings(KEYWORD, "media-supported", &["iso_a4_210x297mm"])
                .string(KEYWORD, "media-default", "iso_a4_210x297mm")
                .strings(KEYWORD, "sides-supported", &["one-sided"])
                .strings(KEYWORD, "print-color-mode-supported", &["monochrome"])
                .strings(KEYWORD, "urf-supported", &["RS100"]);
            reply.boolean("page-ranges-supported", false);
        }
    }
    reply.finished()
}

fn fake_printer(formats: &'static [&'static str]) -> (u16, Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sender, received): (Sender<Vec<u8>>, _) = channel();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = BufReader::new(stream);
            let Some(body) = read_request(&mut stream) else {
                continue;
            };
            let operation = u16::from_be_bytes([body[2], body[3]]);
            if operation == PRINT_JOB {
                sender.send(body).unwrap();
            }
            let reply = answer(operation, formats);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            let mut out = stream.into_inner();
            out.write_all(head.as_bytes()).unwrap();
            out.write_all(&reply).unwrap();
        }
    });
    (port, received)
}

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Daemon {
    child: Child,
    _home: Folder,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_daemon(bus: &SessionBus, name: &str) -> Daemon {
    let home = Folder::new(name);
    let child = Command::new(env!("CARGO_BIN_EXE_bande-printd"))
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .env("XDG_STATE_HOME", &home.0)
        .spawn()
        .unwrap();
    Daemon { child, _home: home }
}

fn document_bytes() -> Vec<u8> {
    let setup = PageSetup::default();
    let mut text = TextSystem::new();
    let mut page = Page::new(&setup);
    page.draw(Item::Fill {
        shape: Shape::rounded(Rect::new(54.0, 54.0, 200.0, 80.0), 8.0),
        paint: Paint::Solid(Color::BLACK),
    });
    page.draw(Item::Text {
        origin: Point::new(54.0, 200.0),
        paragraph: text.paragraph(
            "Printed text",
            TextStyle::new(12.0, Color::BLACK),
            None,
            TextAlign::Start,
        ),
    });
    write("Test", &[page]).unwrap()
}

fn document() -> OwnedFd {
    let bytes = document_bytes();
    let path = std::env::temp_dir().join(format!("bande-print-doc-{}.pdf", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    let file = std::fs::File::open(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    file.into()
}

fn options() -> JobOptions {
    JobOptions {
        media: String::new(),
        landscape: false,
        sides: String::new(),
        color: false,
        copies: 1,
        pages: Vec::new(),
    }
}

fn print_to(formats: &'static [&'static str], name: &str) -> Vec<u8> {
    let bus = SessionBus::start().unwrap();
    let (port, received) = fake_printer(formats);
    let _daemon = start_daemon(&bus, name);
    let connection = bus.connect().unwrap();
    let client = PrintClient::new(&connection).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let printer = loop {
        match client.add_address(&format!("ipp://127.0.0.1:{port}/ipp/print")) {
            Ok(printer) => break printer,
            Err(error) if Instant::now() > deadline => panic!("{error}"),
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    };
    let listed = client.printers().unwrap();
    assert!(listed.iter().any(|known| known.name == "Acme Test"));
    client
        .submit("Test", &printer, &options(), document())
        .unwrap();
    let body = received.recv_timeout(Duration::from_secs(60)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !client.jobs().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the job never finished");
        thread::sleep(Duration::from_millis(50));
    }
    body
}

fn contains(body: &[u8], needle: &[u8]) -> bool {
    body.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn a_printer_that_reads_pdf_gets_the_document_itself() {
    let body = print_to(&["application/pdf", "image/urf"], "bande-printd-pdf");
    assert!(contains(&body, b"application/pdf"));
    assert!(contains(&body, b"%PDF-"));
}

#[test]
fn a_printer_without_pdf_gets_a_page_of_raster() {
    let body = print_to(&["image/urf"], "bande-printd-urf");
    assert!(contains(&body, b"image/urf"));
    assert!(contains(&body, b"UNIRAST\0\0\0\0\x01"));
    assert!(!contains(&body, b"%PDF-"));
}

#[test]
fn text_on_a_toolkit_page_stays_text_in_the_pdf() {
    let folder = Folder::new("bande-printd-text");
    let path = folder.0.join("page.pdf");
    std::fs::write(&path, document_bytes()).unwrap();
    unsafe { std::env::set_var("BANDE_PREVIEW_WORKER", env!("CARGO_BIN_EXE_bande-printd")) };
    let mut session = Session::open(&path).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut text = String::new();
    while Instant::now() < deadline && !text.contains("Printed text") {
        for change in session.take() {
            match change {
                Change::Opened => session.want_text([0]),
                Change::Text(_, glyphs) => {
                    text = glyphs.into_iter().map(|glyph| glyph.text).collect();
                }
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!(text.contains("Printed text"), "found {text:?}");
}
