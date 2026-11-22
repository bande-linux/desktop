use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use rustix::event::{PollFd, PollFlags, poll};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme, StreamOwned};
use sha2::{Digest, Sha256};

use crate::auth::{CHALLENGE, vnc_response};
use crate::clipboard::Clipboard;
use crate::decode::Decoder;
use crate::encode::Frame;
use crate::protocol::{
    Area, ENCODING_DESKTOP_SIZE, ENCODING_EXTENDED_CLIPBOARD, ENCODING_RAW, ENCODING_ZRLE,
    LARGEST_SCREEN, PixelFormat, SECURITY_VENCRYPT, SECURITY_VNC, ServerMessage,
    VENCRYPT_X509_PLAIN, VENCRYPT_X509_VNC, Version, cut_text_message, key_event,
    parse_server_message, parse_version, pointer_event, set_encodings, set_pixel_format,
    update_request,
};
use crate::transport::{Transport, read_array, read_u32};

const SECURITY_NONE: u8 = 1;
const MOST_REASON: usize = 4096;
const MOST_SUBTYPES: usize = 32;
const READ_CHUNK: usize = 256 * 1024;
const MOST_UNPARSED: usize = LARGEST_SCREEN as usize * LARGEST_SCREEN as usize * 4 + 1024;
const CLIENT_CUT_TEXT: u8 = 6;

/// The SHA-256 fingerprint of a DER certificate, as viewers show it: hex pairs split by colons.
pub fn fingerprint(certificate: &[u8]) -> String {
    Sha256::digest(certificate)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// What signs in to a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: String,
}

/// The questions connecting raises, answered by the person.
pub trait Prompter {
    /// The server's certificate, before anything secret is sent: whether to go on.
    fn trust(&mut self, fingerprint: &str) -> bool;
    /// The server offers no encryption: whether to go on without.
    fn unencrypted(&mut self) -> bool;
    /// The password, and with `user` a user name, to sign in with.
    fn credentials(&mut self, user: bool) -> Option<Credentials>;
}

#[derive(Debug)]
pub enum Failure {
    Io(io::Error),
    /// The server would not let this viewer in, for this reason.
    Refused(String),
    /// The server refused the password or the person there declined.
    Denied(String),
    /// The person here stopped.
    Cancelled,
    /// The server speaks nothing this viewer does.
    Unsupported,
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A connection that signed in and knows the screen.
pub struct Session {
    pub transport: Transport,
    pub width: u16,
    pub height: u16,
    pub name: String,
}

#[derive(Debug)]
struct Pinned(Arc<CryptoProvider>);

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
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
            &self.0.signature_verification_algorithms,
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
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn reason(from: &mut impl Read) -> Result<String, Failure> {
    let length = read_u32(from)? as usize;
    if length > MOST_REASON {
        return Err(Failure::Unsupported);
    }
    let mut text = vec![0; length];
    from.read_exact(&mut text)?;
    Ok(String::from_utf8_lossy(&text).into_owned())
}

fn vnc_authentication(transport: &mut Transport, password: &str) -> Result<(), Failure> {
    let challenge = read_array::<CHALLENGE>(transport)?;
    transport.write_all(&vnc_response(password, &challenge))?;
    transport.flush()?;
    Ok(())
}

fn vencrypt(
    mut socket: TcpStream,
    host: &str,
    prompter: &mut dyn Prompter,
) -> Result<Transport, Failure> {
    let version = read_array::<2>(&mut socket)?;
    if version < [0, 2] {
        return Err(Failure::Unsupported);
    }
    socket.write_all(&[0, 2])?;
    if read_array::<1>(&mut socket)? != [0] {
        return Err(Failure::Unsupported);
    }
    let [count] = read_array::<1>(&mut socket)?;
    if usize::from(count) > MOST_SUBTYPES {
        return Err(Failure::Unsupported);
    }
    let subtypes: Vec<u32> = (0..count)
        .map(|_| read_u32(&mut socket))
        .collect::<io::Result<_>>()?;
    let chosen = [VENCRYPT_X509_PLAIN, VENCRYPT_X509_VNC]
        .into_iter()
        .find(|wanted| subtypes.contains(wanted))
        .ok_or(Failure::Unsupported)?;
    socket.write_all(&chosen.to_be_bytes())?;
    if read_array::<1>(&mut socket)? != [1] {
        return Err(Failure::Unsupported);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|error| Failure::Io(io::Error::other(error)))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned(provider)))
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned())
        .or_else(|_| ServerName::try_from("screen-sharing"))
        .map_err(|error| Failure::Io(io::Error::other(error)))?;
    let connection = ClientConnection::new(Arc::new(config), name)
        .map_err(|error| Failure::Io(io::Error::other(error)))?;
    let mut stream = StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    let certificate = stream
        .conn
        .peer_certificates()
        .and_then(|chain| chain.first())
        .ok_or(Failure::Unsupported)?;
    if !prompter.trust(&fingerprint(certificate)) {
        return Err(Failure::Cancelled);
    }
    let mut transport = Transport::Client(Box::new(stream));
    let plain = chosen == VENCRYPT_X509_PLAIN;
    let credentials = prompter.credentials(plain).ok_or(Failure::Cancelled)?;
    match plain {
        true => {
            let mut message = (credentials.user.len() as u32).to_be_bytes().to_vec();
            message.extend_from_slice(&(credentials.password.len() as u32).to_be_bytes());
            message.extend_from_slice(credentials.user.as_bytes());
            message.extend_from_slice(credentials.password.as_bytes());
            transport.write_all(&message)?;
            transport.flush()?;
        }
        false => vnc_authentication(&mut transport, &credentials.password)?,
    }
    Ok(transport)
}

fn unencrypted(
    socket: TcpStream,
    kind: u8,
    prompter: &mut dyn Prompter,
) -> Result<Transport, Failure> {
    if !prompter.unencrypted() {
        return Err(Failure::Cancelled);
    }
    let mut transport = Transport::Plain(socket);
    if kind == SECURITY_VNC {
        let credentials = prompter.credentials(false).ok_or(Failure::Cancelled)?;
        vnc_authentication(&mut transport, &credentials.password)?;
    }
    Ok(transport)
}

/// Signs in to the server at the other end of `socket` (`host` as typed).
pub fn connect(
    mut socket: TcpStream,
    host: &str,
    prompter: &mut dyn Prompter,
) -> Result<Session, Failure> {
    let version = parse_version(&read_array::<12>(&mut socket)?).ok_or(Failure::Unsupported)?;
    socket.write_all(match version {
        Version::V3_3 => b"RFB 003.003\n",
        Version::V3_7 => b"RFB 003.007\n",
        Version::V3_8 => b"RFB 003.008\n",
    })?;
    let kind = match version {
        Version::V3_3 => match read_u32(&mut socket)? {
            0 => return Err(Failure::Refused(reason(&mut socket)?)),
            kind => u8::try_from(kind).map_err(|_| Failure::Unsupported)?,
        },
        Version::V3_7 | Version::V3_8 => {
            let [count] = read_array::<1>(&mut socket)?;
            if count == 0 {
                return Err(Failure::Refused(reason(&mut socket)?));
            }
            let mut offered = vec![0; usize::from(count)];
            socket.read_exact(&mut offered)?;
            let kind = [SECURITY_VENCRYPT, SECURITY_VNC, SECURITY_NONE]
                .into_iter()
                .find(|wanted| offered.contains(wanted))
                .ok_or(Failure::Unsupported)?;
            socket.write_all(&[kind])?;
            kind
        }
    };
    let mut transport = match kind {
        SECURITY_VENCRYPT => vencrypt(socket, host, prompter)?,
        SECURITY_VNC | SECURITY_NONE => unencrypted(socket, kind, prompter)?,
        _ => return Err(Failure::Unsupported),
    };
    if version == Version::V3_8 || kind != SECURITY_NONE {
        let result = read_u32(&mut transport)?;
        if result != 0 {
            return Err(Failure::Denied(match version {
                Version::V3_8 => reason(&mut transport)?,
                _ => String::new(),
            }));
        }
    }
    transport.write_all(&[1])?;
    transport.flush()?;
    let init = read_array::<20>(&mut transport)?;
    let width = u16::from_be_bytes([init[0], init[1]]);
    let height = u16::from_be_bytes([init[2], init[3]]);
    if width > LARGEST_SCREEN || height > LARGEST_SCREEN {
        return Err(Failure::Unsupported);
    }
    let name = reason(&mut transport)?;
    Ok(Session {
        transport,
        width,
        height,
        name,
    })
}

/// What a viewer sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Pointer { x: u16, y: u16, buttons: u8 },
    Key { keysym: u32, down: bool },
    Copy(String),
}

/// Where a viewer shows what the server sends.
pub trait Screen {
    /// The screen has a new size; `frame` is black until painted.
    fn resized(&mut self, frame: &Frame);
    /// These areas of `frame` changed.
    fn painted(&mut self, frame: &Frame, areas: &[Area]);
    /// The server's clipboard has this text now.
    fn copied(&mut self, text: String);
}

fn send(transport: &mut Transport, message: &[u8]) -> io::Result<()> {
    transport.write_all(message)?;
    transport.flush()
}

/// Shows the server's screen and sends what `commands` holds, waking when `wake` is readable,
/// until the server or the commands' sender goes away.
pub fn view(
    session: Session,
    commands: &Receiver<Command>,
    wake: &UnixStream,
    screen: &mut dyn Screen,
) -> Result<(), Failure> {
    let Session {
        mut transport,
        width,
        height,
        ..
    } = session;
    let mut frame = Frame {
        width: u32::from(width),
        height: u32::from(height),
        pixels: vec![0; usize::from(width) * usize::from(height)],
    };
    screen.resized(&frame);
    send(&mut transport, &set_pixel_format(PixelFormat::NATIVE))?;
    send(
        &mut transport,
        &set_encodings(&[
            ENCODING_ZRLE,
            ENCODING_RAW,
            ENCODING_DESKTOP_SIZE,
            ENCODING_EXTENDED_CLIPBOARD,
        ]),
    )?;
    send(&mut transport, &update_request(false, frame.area()))?;
    let mut decoder = Decoder::default();
    let mut clipboard = Clipboard::default();
    let mut unparsed = Vec::new();
    let mut chunk = vec![0; READ_CHUNK];
    loop {
        if !transport.buffered() {
            let mut fds = [
                PollFd::new(transport.socket(), PollFlags::IN),
                PollFd::new(wake, PollFlags::IN),
            ];
            poll(&mut fds, None).map_err(io::Error::from)?;
            let readable = !fds[0].revents().is_empty();
            if !fds[1].revents().is_empty() {
                let mut drained = [0; 64];
                if let Err(error) = rustix::io::read(wake.as_fd(), &mut drained) {
                    tracing::debug!("cannot clear the viewer's waker: {error}");
                }
                loop {
                    let message = match commands.try_recv() {
                        Ok(Command::Pointer { x, y, buttons }) => pointer_event(buttons, x, y),
                        Ok(Command::Key { keysym, down }) => key_event(keysym, down),
                        Ok(Command::Copy(text)) => {
                            cut_text_message(CLIENT_CUT_TEXT, &clipboard.copied_here(text))
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return Ok(()),
                    };
                    send(&mut transport, &message)?;
                }
            }
            if !readable {
                continue;
            }
        }
        let read = transport.read(&mut chunk)?;
        if read == 0 {
            return Err(Failure::Io(io::ErrorKind::UnexpectedEof.into()));
        }
        unparsed.extend_from_slice(&chunk[..read]);
        let mut used = 0;
        loop {
            let size = (frame.width as u16, frame.height as u16);
            let parsed = parse_server_message(&unparsed[used..], size)
                .map_err(|refused| Failure::Io(io::Error::other(format!("{refused:?}"))))?;
            let Some((message, length)) = parsed else {
                break;
            };
            used += length;
            match message {
                ServerMessage::Update(rectangles) => {
                    let mut areas = Vec::new();
                    for rectangle in &rectangles {
                        if rectangle.encoding == ENCODING_DESKTOP_SIZE {
                            frame = Frame {
                                width: u32::from(rectangle.area.width),
                                height: u32::from(rectangle.area.height),
                                pixels: vec![
                                    0;
                                    usize::from(rectangle.area.width)
                                        * usize::from(rectangle.area.height)
                                ],
                            };
                            screen.resized(&frame);
                            areas.clear();
                            continue;
                        }
                        decoder.paint(&mut frame, rectangle).map_err(|_| {
                            Failure::Io(io::Error::other("the server sent a broken picture"))
                        })?;
                        areas.push(rectangle.area);
                    }
                    screen.painted(&frame, &areas);
                    send(&mut transport, &update_request(true, frame.area()))?;
                }
                ServerMessage::CutText(text) => {
                    let outcome = clipboard.received(text);
                    if let Some(reply) = outcome.reply {
                        send(&mut transport, &cut_text_message(CLIENT_CUT_TEXT, &reply))?;
                    }
                    if let Some(text) = outcome.copied {
                        screen.copied(text);
                    }
                }
                ServerMessage::Bell | ServerMessage::Colours => {}
            }
        }
        unparsed.drain(..used);
        if unparsed.len() > MOST_UNPARSED {
            return Err(Failure::Io(io::Error::other(
                "the server sent too much at once",
            )));
        }
    }
}
