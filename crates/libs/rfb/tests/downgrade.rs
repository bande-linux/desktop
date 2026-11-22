#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use bande_rfb::client::{self, Credentials, Failure, Prompter};

#[derive(Default)]
struct Asked {
    unencrypted: bool,
    password_unencrypted: Option<bool>,
}

impl Prompter for Asked {
    fn trust(&mut self, _fingerprint: &str) -> bool {
        panic!("a plain server shows no certificate")
    }

    fn unencrypted(&mut self) -> bool {
        self.unencrypted = true;
        true
    }

    fn credentials(&mut self, _user: bool, encrypted: bool) -> Option<Credentials> {
        self.password_unencrypted = Some(!encrypted);
        None
    }
}

fn plain_server() -> (u16, thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        socket.write_all(b"RFB 003.008\n").unwrap();
        let mut version = [0; 12];
        socket.read_exact(&mut version).unwrap();
        socket.write_all(&[1, 2]).unwrap();
        socket.write_all(&[7; 16]).unwrap();
        let mut after = Vec::new();
        let _ = socket.read_to_end(&mut after);
        after
    });
    (port, served)
}

fn connected(port: u16) -> TcpStream {
    let socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    socket
}

#[test]
fn a_computer_that_offered_tls_is_refused_when_it_offers_none_and_nothing_is_sent() {
    let (port, served) = plain_server();
    let mut asked = Asked::default();
    match client::connect(connected(port), "127.0.0.1", true, &mut asked) {
        Err(Failure::Downgraded) => {}
        Err(other) => panic!("expected a refused downgrade, got {other:?}"),
        Ok(_) => panic!("a downgraded server was used"),
    }
    assert!(!asked.unencrypted && asked.password_unencrypted.is_none());
    assert!(
        served.join().unwrap().is_empty(),
        "the viewer sent nothing after its version"
    );

    let (port, served) = plain_server();
    let mut asked = Asked::default();
    assert!(matches!(
        client::connect(connected(port), "127.0.0.1", false, &mut asked),
        Err(Failure::Cancelled)
    ));
    assert!(
        asked.unencrypted,
        "an unknown plain server is used only after asking"
    );
    assert_eq!(
        asked.password_unencrypted,
        Some(true),
        "the prompter is told the password would travel unencrypted"
    );
    assert_eq!(
        served.join().unwrap(),
        [2],
        "only the choice was sent, no password"
    );
}
