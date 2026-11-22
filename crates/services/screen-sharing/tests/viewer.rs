#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bande_rfb::client::{self, Command, Credentials, Failure, Prompter, fingerprint};
use bande_screen_sharing::rfb::identity::Identity;
use bande_screen_sharing::rfb::{Frame, Host, Input, Screen, Viewing, serve};

const PASSWORD: &str = "open sesame";
const PATIENCE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Sent {
    Pointer(u32, u32),
    Button(u8, bool),
    Key(u32, bool),
    Copied(String),
}

#[derive(Default)]
struct Recorded(Mutex<Vec<Sent>>);

impl Input for Recorded {
    fn pointer(&self, x: u32, y: u32) {
        self.0.lock().unwrap().push(Sent::Pointer(x, y));
    }

    fn button(&self, button: u8, pressed: bool) {
        self.0.lock().unwrap().push(Sent::Button(button, pressed));
    }

    fn scroll(&self, _x: i32, _y: i32) {}

    fn key(&self, keysym: u32, down: bool) {
        self.0.lock().unwrap().push(Sent::Key(keysym, down));
    }
}

struct TestHost {
    screen: Arc<Screen>,
    input: Arc<Recorded>,
    older_viewers: AtomicBool,
}

impl Host for TestHost {
    fn password(&self) -> Option<String> {
        Some(PASSWORD.to_owned())
    }

    fn older_viewers(&self) -> bool {
        self.older_viewers.load(Ordering::SeqCst)
    }

    fn admit(&self, _peer: IpAddr) -> bool {
        true
    }

    fn view(&self, _peer: IpAddr) -> Option<Viewing> {
        Some(Viewing {
            screen: Arc::clone(&self.screen),
            input: Arc::clone(&self.input) as _,
            kept: Arc::new(()),
        })
    }

    fn copied(&self, text: String) {
        self.input.0.lock().unwrap().push(Sent::Copied(text));
    }

    fn name(&self) -> String {
        "test screen".to_owned()
    }
}

struct Answers {
    password: &'static str,
    seen: Option<String>,
}

impl Prompter for Answers {
    fn trust(&mut self, fingerprint: &str) -> bool {
        self.seen = Some(fingerprint.to_owned());
        true
    }

    fn unencrypted(&mut self) -> bool {
        panic!("the server offers encryption")
    }

    fn credentials(&mut self, _user: bool, encrypted: bool) -> Option<Credentials> {
        assert!(encrypted, "the password goes over TLS");
        Some(Credentials {
            user: "guest".to_owned(),
            password: self.password.to_owned(),
        })
    }
}

#[derive(Debug)]
enum Shown {
    Picture(Frame),
    Copied(String),
}

struct Window(Sender<Shown>);

impl client::Screen for Window {
    fn resized(&mut self, _frame: &Frame) {}

    fn painted(&mut self, frame: &Frame, _areas: &[bande_rfb::protocol::Area]) {
        let _ = self.0.send(Shown::Picture(frame.clone()));
    }

    fn copied(&mut self, text: String) {
        let _ = self.0.send(Shown::Copied(text));
    }
}

fn picture() -> Frame {
    Frame {
        width: 2,
        height: 2,
        pixels: vec![0x11_2233, 0x44_5566, 0x77_8899, 0xaa_bbcc],
    }
}

fn start(older_viewers: bool) -> (u16, Vec<u8>, Arc<TestHost>) {
    let identity = Identity::generate("test").unwrap();
    let tls = identity.tls().unwrap();
    let screen = Arc::new(Screen::default());
    screen.show(picture());
    let host = Arc::new(TestHost {
        screen,
        input: Arc::default(),
        older_viewers: AtomicBool::new(older_viewers),
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = Arc::clone(&host) as Arc<dyn Host>;
    thread::spawn(move || serve(&listener, served, tls, |peer| peer.is_loopback()));
    (port, identity.certificate, host)
}

fn connected(port: u16) -> TcpStream {
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket.set_read_timeout(Some(PATIENCE)).unwrap();
    socket
}

fn security_types(port: u16) -> (TcpStream, Vec<u8>) {
    let mut socket = connected(port);
    let mut version = [0; 12];
    socket.read_exact(&mut version).unwrap();
    socket.write_all(b"RFB 003.008\n").unwrap();
    let mut count = [0];
    socket.read_exact(&mut count).unwrap();
    let mut types = vec![0; usize::from(count[0])];
    socket.read_exact(&mut types).unwrap();
    (socket, types)
}

fn wait_for(host: &TestHost, sent: &[Sent]) {
    let deadline = Instant::now() + PATIENCE;
    while host.input.0.lock().unwrap().as_slice() != sent {
        assert!(
            Instant::now() < deadline,
            "the viewer's input never arrived"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn next(shown: &Receiver<Shown>) -> Shown {
    shown.recv_timeout(PATIENCE).unwrap()
}

#[test]
fn the_viewer_signs_in_over_tls_sees_and_controls_the_screen_and_shares_the_clipboard() {
    let (port, certificate, host) = start(false);
    let mut answers = Answers {
        password: PASSWORD,
        seen: None,
    };
    let session = client::connect(connected(port), "127.0.0.1", false, &mut answers).unwrap();
    assert_eq!(answers.seen, Some(fingerprint(&certificate)));
    assert_eq!(
        (session.width, session.height, session.name.as_str()),
        (2, 2, "test screen")
    );

    let (commands, received) = mpsc::channel();
    let (wake, woken) = UnixStream::pair().unwrap();
    let (shown, seen) = mpsc::channel();
    let viewer = thread::spawn(move || {
        client::view(session, &received, &woken, &mut Window(shown)).unwrap();
    });
    let send = |command: Command| {
        commands.send(command).unwrap();
        (&wake).write_all(&[1]).unwrap();
    };
    match next(&seen) {
        Shown::Picture(frame) => assert_eq!(frame, picture()),
        other => panic!("expected the screen, got {other:?}"),
    }

    send(Command::Pointer {
        x: 1,
        y: 1,
        buttons: 1,
    });
    send(Command::Key {
        keysym: 0x61,
        down: true,
    });
    send(Command::Copy("grüße ☃".to_owned()));
    wait_for(
        &host,
        &[
            Sent::Pointer(1, 1),
            Sent::Button(0, true),
            Sent::Key(0x61, true),
            Sent::Copied("grüße ☃".to_owned()),
        ],
    );

    host.screen.copy("κόσμε\nline".to_owned());
    loop {
        match next(&seen) {
            Shown::Copied(text) => {
                assert_eq!(text, "κόσμε\nline");
                break;
            }
            Shown::Picture(_) => {}
        }
    }
    drop(commands);
    (&wake).write_all(&[1]).unwrap();
    viewer.join().unwrap();
    wait_for(
        &host,
        &[
            Sent::Pointer(1, 1),
            Sent::Button(0, true),
            Sent::Key(0x61, true),
            Sent::Copied("grüße ☃".to_owned()),
            Sent::Key(0x61, false),
            Sent::Button(0, false),
        ],
    );
}

#[test]
fn a_wrong_password_is_refused_and_older_viewers_need_permission() {
    let (port, _, host) = start(false);
    let (mut socket, types) = security_types(port);
    assert_eq!(types, [19], "plain VNC authentication is off by default");
    socket.write_all(&[19]).unwrap();
    let mut version = [0; 2];
    socket.read_exact(&mut version).unwrap();
    socket.write_all(&[0, 2]).unwrap();
    let mut subtypes = [0; 6];
    socket.read_exact(&mut subtypes).unwrap();
    assert_eq!(
        subtypes,
        [0, 1, 0, 0, 1, 6],
        "only X509Plain, with the whole password, until older viewers are allowed"
    );
    let mut busy = Answers {
        password: PASSWORD,
        seen: None,
    };
    match client::connect(connected(port), "127.0.0.1", false, &mut busy) {
        Err(Failure::Refused(reason)) => assert_eq!(
            reason, "This computer is already connecting.",
            "one handshake at a time from one address, and the viewer is told why"
        ),
        Err(other) => panic!("expected a refusal, got {other:?}"),
        Ok(_) => panic!("a second handshake from one address got in"),
    }
    drop(socket);
    thread::sleep(Duration::from_millis(200));
    let mut answers = Answers {
        password: "guessed",
        seen: None,
    };
    match client::connect(connected(port), "127.0.0.1", false, &mut answers) {
        Err(Failure::Denied(reason)) => assert_eq!(reason, "Wrong password."),
        Err(other) => panic!("expected a refused password, got {other:?}"),
        Ok(_) => panic!("a wrong password got in"),
    }

    host.older_viewers.store(true, Ordering::SeqCst);
    let (mut socket, types) = security_types(port);
    assert_eq!(types, [19, 2]);
    socket.write_all(&[2]).unwrap();
    let mut challenge = [0; 16];
    socket.read_exact(&mut challenge).unwrap();
    socket.write_all(&[0; 16]).unwrap();
    let mut result = [0; 4];
    socket.read_exact(&mut result).unwrap();
    assert_eq!(result, [0, 0, 0, 1], "the wrong password is refused");
    assert!(host.input.0.lock().unwrap().is_empty());
}
