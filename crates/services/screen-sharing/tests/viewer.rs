use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bande_screen_sharing::rfb::identity::Identity;
use bande_screen_sharing::rfb::{Frame, Host, Input, Screen, Viewing, serve};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};

const PASSWORD: &str = "open sesame";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Sent {
    Pointer(u32, u32, u8),
    Key(u32, bool),
}

#[derive(Default)]
struct Recorded(Mutex<Vec<Sent>>);

impl Input for Recorded {
    fn pointer(&self, x: u32, y: u32, buttons: u8) {
        self.0.lock().unwrap().push(Sent::Pointer(x, y, buttons));
    }

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

    fn name(&self) -> String {
        "test screen".to_owned()
    }
}

#[derive(Debug)]
struct Pinned(Vec<u8>, Arc<CryptoProvider>);

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match end_entity.as_ref() == self.0 {
            true => Ok(ServerCertVerified::assertion()),
            false => Err(rustls::Error::General("another certificate".into())),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            certificate,
            signed,
            &self.1.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            certificate,
            signed,
            &self.1.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.1.signature_verification_algorithms.supported_schemes()
    }
}

fn start(older_viewers: bool) -> (u16, Vec<u8>, Arc<TestHost>) {
    let identity = Identity::generate("test").unwrap();
    let tls = identity.tls().unwrap();
    let screen = Arc::new(Screen::default());
    screen.show(Frame {
        width: 2,
        height: 2,
        pixels: vec![0x11_2233, 0x44_5566, 0x77_8899, 0xaa_bbcc],
    });
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

fn read<const N: usize>(from: &mut impl Read) -> [u8; N] {
    let mut bytes = [0; N];
    from.read_exact(&mut bytes).unwrap();
    bytes
}

fn security_types(port: u16) -> (TcpStream, Vec<u8>) {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    assert_eq!(&read::<12>(&mut socket), b"RFB 003.008\n");
    socket.write_all(b"RFB 003.008\n").unwrap();
    let [count] = read::<1>(&mut socket);
    let mut types = vec![0; usize::from(count)];
    socket.read_exact(&mut types).unwrap();
    (socket, types)
}

fn wait_for(host: &TestHost, sent: &[Sent]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while host.input.0.lock().unwrap().as_slice() != sent {
        assert!(
            Instant::now() < deadline,
            "the viewer's input never arrived"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_viewer_with_the_password_sees_and_controls_the_screen_over_tls() {
    let (port, certificate, host) = start(false);
    let (mut socket, types) = security_types(port);
    assert_eq!(
        types,
        [19],
        "only VeNCrypt is offered unless older viewers are allowed"
    );
    socket.write_all(&[19]).unwrap();
    assert_eq!(read::<2>(&mut socket), [0, 2]);
    socket.write_all(&[0, 2]).unwrap();
    assert_eq!(read::<2>(&mut socket), [0, 2]);
    assert_eq!(read::<8>(&mut socket), [0, 0, 1, 6, 0, 0, 1, 5]);
    socket.write_all(&262u32.to_be_bytes()).unwrap();
    assert_eq!(read::<1>(&mut socket), [1]);

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned(certificate, provider)))
        .with_no_client_auth();
    let connection =
        ClientConnection::new(Arc::new(config), ServerName::try_from("viewer").unwrap()).unwrap();
    let mut tls = StreamOwned::new(connection, socket);
    tls.write_all(&5u32.to_be_bytes()).unwrap();
    tls.write_all(&(PASSWORD.len() as u32).to_be_bytes())
        .unwrap();
    tls.write_all(b"guest").unwrap();
    tls.write_all(PASSWORD.as_bytes()).unwrap();
    assert_eq!(
        read::<4>(&mut tls),
        [0, 0, 0, 0],
        "the password is accepted"
    );

    tls.write_all(&[1]).unwrap();
    let init = read::<24>(&mut tls);
    assert_eq!(&init[..4], &[0, 2, 0, 2], "a 2×2 screen");
    let mut name = vec![0; u32::from_be_bytes([init[20], init[21], init[22], init[23]]) as usize];
    tls.read_exact(&mut name).unwrap();
    assert_eq!(name, b"test screen");

    tls.write_all(&[2, 0, 0, 1, 0, 0, 0, 0]).unwrap();
    tls.write_all(&[3, 0, 0, 0, 0, 0, 0, 2, 0, 2]).unwrap();
    assert_eq!(read::<4>(&mut tls), [0, 0, 0, 1], "one rectangle");
    assert_eq!(read::<12>(&mut tls), [0, 0, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0]);
    assert_eq!(
        read::<16>(&mut tls),
        [
            0x33, 0x22, 0x11, 0, 0x66, 0x55, 0x44, 0, 0x99, 0x88, 0x77, 0, 0xcc, 0xbb, 0xaa, 0
        ]
    );

    tls.write_all(&[5, 1, 0, 1, 0, 1]).unwrap();
    tls.write_all(&[4, 1, 0, 0, 0, 0, 0, 0x61]).unwrap();
    wait_for(&host, &[Sent::Pointer(1, 1, 1), Sent::Key(0x61, true)]);
}

#[test]
fn a_wrong_password_is_refused_and_older_viewers_need_permission() {
    let (port, _, host) = start(false);
    let (_socket, types) = security_types(port);
    assert!(
        !types.contains(&2),
        "plain VNC authentication is off by default"
    );

    host.older_viewers.store(true, Ordering::SeqCst);
    let (mut socket, types) = security_types(port);
    assert_eq!(types, [19, 2]);
    socket.write_all(&[2]).unwrap();
    let _challenge = read::<16>(&mut socket);
    socket.write_all(&[0; 16]).unwrap();
    assert_eq!(
        read::<4>(&mut socket),
        [0, 0, 0, 1],
        "the wrong password is refused"
    );
    let length = u32::from_be_bytes(read::<4>(&mut socket));
    let mut reason = vec![0; length as usize];
    socket.read_exact(&mut reason).unwrap();
    assert_eq!(reason, b"Wrong password.");
    assert!(host.input.0.lock().unwrap().is_empty());
}
