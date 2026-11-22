use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use ring::rand::{SecureRandom, SystemRandom};
use rustix::event::{PollFd, PollFlags, poll};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use super::auth::{Attempts, CHALLENGE, same, vnc_response};
use super::encode::{Encoder, changed_areas};
use super::protocol::{
    Area, ClientMessage, ENCODING_DESKTOP_SIZE, SECURITY_VENCRYPT, SECURITY_VNC,
    VENCRYPT_X509_PLAIN, VENCRYPT_X509_VNC, VERSION, Version, parse_message, parse_version,
    rectangle_header, server_init, update_header,
};
use super::{Frame, Host, Viewing};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const FAILURE_DELAY: Duration = Duration::from_secs(1);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const MOST_VIEWERS: usize = 4;
const MOST_CREDENTIAL: usize = 1024;
const MOST_UNPARSED: usize = super::protocol::MOST_CUT_TEXT + 64 * 1024;
const READ_CHUNK: usize = 64 * 1024;
const FAILED: u32 = 1;

enum Transport {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
}

impl Transport {
    fn socket(&self) -> &TcpStream {
        match self {
            Self::Plain(socket) => socket,
            Self::Tls(stream) => &stream.sock,
        }
    }

    fn buffered(&mut self) -> bool {
        match self {
            Self::Plain(_) => false,
            Self::Tls(stream) => stream
                .conn
                .process_new_packets()
                .is_ok_and(|state| state.plaintext_bytes_to_read() > 0),
        }
    }
}

impl Read for Transport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(socket) => socket.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

fn read_array<const N: usize>(from: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    from.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_u32(from: &mut impl Read) -> io::Result<u32> {
    read_array::<4>(from).map(u32::from_be_bytes)
}

fn refused(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, why.to_owned())
}

fn with_reason(code: Option<u32>, reason: &str) -> Vec<u8> {
    let mut message = code.map(u32::to_be_bytes).unwrap_or_default().to_vec();
    message.extend_from_slice(&(reason.len() as u32).to_be_bytes());
    message.extend_from_slice(reason.as_bytes());
    message
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Checked {
    Right,
    Wrong,
}

fn vnc_authentication(transport: &mut Transport, password: &str) -> io::Result<Checked> {
    let mut challenge = [0; CHALLENGE];
    SystemRandom::new()
        .fill(&mut challenge)
        .map_err(|_| io::Error::other("no random numbers for the challenge"))?;
    transport.write_all(&challenge)?;
    transport.flush()?;
    let response = read_array::<CHALLENGE>(transport)?;
    Ok(match same(&response, &vnc_response(password, &challenge)) {
        true => Checked::Right,
        false => Checked::Wrong,
    })
}

fn vencrypt(
    mut socket: TcpStream,
    tls: &Arc<ServerConfig>,
    password: &str,
) -> io::Result<(Transport, Checked)> {
    socket.write_all(&[0, 2])?;
    let version = read_array::<2>(&mut socket)?;
    if version != [0, 2] {
        socket.write_all(&[1])?;
        return Err(refused("the viewer speaks another VeNCrypt version"));
    }
    socket.write_all(&[0, 2])?;
    socket.write_all(&VENCRYPT_X509_PLAIN.to_be_bytes())?;
    socket.write_all(&VENCRYPT_X509_VNC.to_be_bytes())?;
    let chosen = read_u32(&mut socket)?;
    if chosen != VENCRYPT_X509_PLAIN && chosen != VENCRYPT_X509_VNC {
        socket.write_all(&[0])?;
        return Err(refused("the viewer chose no offered VeNCrypt type"));
    }
    socket.write_all(&[1])?;
    let connection = ServerConnection::new(Arc::clone(tls)).map_err(io::Error::other)?;
    let mut stream = StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    let mut transport = Transport::Tls(Box::new(stream));
    if chosen == VENCRYPT_X509_VNC {
        let checked = vnc_authentication(&mut transport, password)?;
        return Ok((transport, checked));
    }
    let user_length = read_u32(&mut transport)? as usize;
    let password_length = read_u32(&mut transport)? as usize;
    if user_length > MOST_CREDENTIAL || password_length > MOST_CREDENTIAL {
        return Err(refused("the viewer sent too long a user name or password"));
    }
    let mut credentials = vec![0; user_length + password_length];
    transport.read_exact(&mut credentials)?;
    let checked = match same(&credentials[user_length..], password.as_bytes()) {
        true => Checked::Right,
        false => Checked::Wrong,
    };
    Ok((transport, checked))
}

fn handshake(
    mut socket: TcpStream,
    peer: IpAddr,
    host: &dyn Host,
    tls: &Arc<ServerConfig>,
    attempts: &Attempts,
) -> io::Result<Option<Transport>> {
    socket.write_all(VERSION)?;
    let version = parse_version(&read_array::<12>(&mut socket)?)
        .ok_or_else(|| refused("the viewer speaks no known RFB version"))?;
    let password = host.password();
    let older_viewers = host.older_viewers();
    let refusal = match (&password, attempts.blocked(peer)) {
        (_, true) => Some("Too many wrong passwords. Try again later."),
        (None, _) => Some("Screen sharing has no password yet."),
        (Some(_), false) => None,
    };
    let Some(password) = password.filter(|_| refusal.is_none()) else {
        let reason = refusal.unwrap_or_default();
        let code = (version == Version::V3_3).then_some(0);
        socket.write_all(&match code {
            Some(_) => with_reason(code, reason),
            None => [vec![0], with_reason(None, reason)].concat(),
        })?;
        return Ok(None);
    };
    let (mut transport, checked) = match version {
        Version::V3_3 if older_viewers => {
            socket.write_all(&u32::from(SECURITY_VNC).to_be_bytes())?;
            let mut transport = Transport::Plain(socket);
            let checked = vnc_authentication(&mut transport, &password)?;
            (transport, checked)
        }
        Version::V3_3 => {
            socket.write_all(&with_reason(
                Some(0),
                "This viewer is too old. Allow older viewers in Settings to use it.",
            ))?;
            return Ok(None);
        }
        Version::V3_7 | Version::V3_8 => {
            let offered: &[u8] = match older_viewers {
                true => &[2, SECURITY_VENCRYPT, SECURITY_VNC],
                false => &[1, SECURITY_VENCRYPT],
            };
            socket.write_all(offered)?;
            let [chosen] = read_array::<1>(&mut socket)?;
            match chosen {
                SECURITY_VENCRYPT => vencrypt(socket, tls, &password)?,
                SECURITY_VNC if older_viewers => {
                    let mut transport = Transport::Plain(socket);
                    let checked = vnc_authentication(&mut transport, &password)?;
                    (transport, checked)
                }
                _ => return Err(refused("the viewer chose no offered security type")),
            }
        }
    };
    let failure = |transport: &mut Transport, reason: &str| -> io::Result<()> {
        match version {
            Version::V3_8 => transport.write_all(&with_reason(Some(FAILED), reason))?,
            Version::V3_3 | Version::V3_7 => transport.write_all(&FAILED.to_be_bytes())?,
        }
        transport.flush()
    };
    if checked == Checked::Wrong {
        attempts.failed(peer);
        tracing::info!("a viewer at {peer} gave a wrong password");
        thread::sleep(FAILURE_DELAY);
        failure(&mut transport, "Wrong password.")?;
        return Ok(None);
    }
    if !host.admit(peer) {
        tracing::info!("the viewer at {peer} was not let in");
        failure(
            &mut transport,
            "The person at this computer didn’t let you in.",
        )?;
        return Ok(None);
    }
    transport.write_all(&0u32.to_be_bytes())?;
    transport.flush()?;
    read_array::<1>(&mut transport)?;
    Ok(Some(transport))
}

struct Request {
    incremental: bool,
    area: Area,
}

fn union(first: Area, second: Area) -> Area {
    let right = (u32::from(first.x) + u32::from(first.width))
        .max(u32::from(second.x) + u32::from(second.width));
    let bottom = (u32::from(first.y) + u32::from(first.height))
        .max(u32::from(second.y) + u32::from(second.height));
    let x = first.x.min(second.x);
    let y = first.y.min(second.y);
    Area {
        x,
        y,
        width: u16::try_from(right - u32::from(x)).unwrap_or(u16::MAX),
        height: u16::try_from(bottom - u32::from(y)).unwrap_or(u16::MAX),
    }
}

fn clip_to(area: Area, size: (u16, u16)) -> Option<Area> {
    let right = (u32::from(area.x) + u32::from(area.width)).min(u32::from(size.0));
    let bottom = (u32::from(area.y) + u32::from(area.height)).min(u32::from(size.1));
    (u32::from(area.x) < right && u32::from(area.y) < bottom).then(|| Area {
        width: (right - u32::from(area.x)) as u16,
        height: (bottom - u32::from(area.y)) as u16,
        ..area
    })
}

struct Viewer {
    transport: Transport,
    viewing: Viewing,
    encoder: Encoder,
    request: Option<Request>,
    sent: Option<Arc<Frame>>,
    checked: u64,
    size: (u16, u16),
    unparsed: Vec<u8>,
}

impl Viewer {
    fn update(&mut self, frame: &Arc<Frame>) -> io::Result<()> {
        let Some(request) = self.request.take() else {
            return Ok(());
        };
        let mut rectangles = Vec::new();
        let mut body = Vec::new();
        let area = frame.area();
        let resized = (area.width, area.height) != self.size;
        let mut wanted = request.area;
        if resized && self.encoder.desktop_size {
            rectangle_header(area, ENCODING_DESKTOP_SIZE, &mut body);
            rectangles.push(area);
            self.size = (area.width, area.height);
            wanted = area;
        }
        let incremental = request.incremental && !resized;
        if incremental
            && self
                .sent
                .as_ref()
                .is_some_and(|sent| Arc::ptr_eq(sent, frame))
        {
            self.request = Some(request);
            return Ok(());
        }
        let areas = match incremental {
            true => changed_areas(self.sent.as_deref(), frame, wanted),
            false => vec![wanted],
        };
        let areas: Vec<Area> = areas
            .into_iter()
            .filter_map(|area| clip_to(area, self.size))
            .collect();
        if areas.is_empty() && rectangles.is_empty() {
            if incremental {
                self.sent = Some(Arc::clone(frame));
                self.request = Some(request);
                return Ok(());
            }
            self.sent = Some(Arc::clone(frame));
            return self.send(0, &body);
        }
        for area in &areas {
            self.encoder.encode(frame, *area, &mut body)?;
        }
        self.sent = Some(Arc::clone(frame));
        let count = u16::try_from(rectangles.len() + areas.len()).unwrap_or(u16::MAX);
        self.send(count, &body)
    }

    fn send(&mut self, rectangles: u16, body: &[u8]) -> io::Result<()> {
        let mut message = Vec::with_capacity(body.len() + 4);
        update_header(rectangles, &mut message);
        message.extend_from_slice(body);
        self.transport.write_all(&message)?;
        self.transport.flush()
    }

    fn take(&mut self, message: ClientMessage) -> io::Result<()> {
        match message {
            ClientMessage::SetPixelFormat(format) if format.is_usable() => {
                self.encoder.format = format;
                self.sent = None;
            }
            ClientMessage::SetPixelFormat(_) => {
                return Err(refused(
                    "the viewer asked for a pixel format without true colour",
                ));
            }
            ClientMessage::SetEncodings(encodings) => self.encoder.set_encodings(&encodings),
            ClientMessage::UpdateRequest { incremental, area } => {
                self.checked = 0;
                self.request = Some(match self.request.take() {
                    Some(earlier) => Request {
                        incremental: incremental && earlier.incremental,
                        area: union(earlier.area, area),
                    },
                    None => Request { incremental, area },
                });
            }
            ClientMessage::Key { down, keysym } => self.viewing.input.key(keysym, down),
            ClientMessage::Pointer { buttons, x, y } => {
                self.viewing
                    .input
                    .pointer(u32::from(x), u32::from(y), buttons);
            }
            ClientMessage::CutText(_) => {}
        }
        Ok(())
    }

    fn receive(&mut self) -> io::Result<()> {
        let mut chunk = vec![0; READ_CHUNK];
        let read = self.transport.read(&mut chunk)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.unparsed.extend_from_slice(&chunk[..read]);
        let mut used = 0;
        loop {
            match parse_message(&self.unparsed[used..]) {
                Ok(Some((message, length))) => {
                    used += length;
                    self.take(message)?;
                }
                Ok(None) => break,
                Err(malformed) => {
                    return Err(refused(&format!("the viewer sent {malformed:?}")));
                }
            }
        }
        self.unparsed.drain(..used);
        if self.unparsed.len() > MOST_UNPARSED {
            return Err(refused("the viewer sent too much at once"));
        }
        Ok(())
    }

    fn run(&mut self) -> io::Result<()> {
        let waker = self.viewing.screen.waker()?;
        loop {
            let latest = self.viewing.screen.latest();
            if latest.ended {
                return Ok(());
            }
            if let Some(frame) = &latest.frame
                && self.request.is_some()
                && latest.serial != self.checked
            {
                self.update(frame)?;
                self.checked = latest.serial;
            }
            if !self.transport.buffered() {
                let mut fds = [
                    PollFd::new(self.transport.socket(), PollFlags::IN),
                    PollFd::new(&*waker, PollFlags::IN),
                ];
                poll(&mut fds, None)?;
                let readable = !fds[0].revents().is_empty();
                if !fds[1].revents().is_empty() {
                    let mut count = [0; 8];
                    if let Err(error) = rustix::io::read(waker.as_fd(), &mut count) {
                        tracing::debug!("cannot clear the frame waker: {error}");
                    }
                }
                if !readable {
                    continue;
                }
            }
            self.receive()?;
        }
    }
}

fn serve_viewer(
    socket: TcpStream,
    peer: IpAddr,
    host: &dyn Host,
    tls: &Arc<ServerConfig>,
    attempts: &Attempts,
) -> io::Result<()> {
    socket.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    socket.set_write_timeout(Some(WRITE_TIMEOUT))?;
    socket.set_nodelay(true)?;
    let watched = socket.try_clone()?;
    let (finished, unfinished) = mpsc::channel::<()>();
    thread::Builder::new()
        .name("handshake".into())
        .spawn(move || {
            if unfinished.recv_timeout(HANDSHAKE_DEADLINE) == Err(RecvTimeoutError::Timeout) {
                let _ = watched.shutdown(Shutdown::Both);
            }
        })?;
    let shaken = handshake(socket, peer, host, tls, attempts);
    drop(finished);
    let Some(mut transport) = shaken? else {
        return Ok(());
    };
    let Some(viewing) = host.view(peer) else {
        return Err(io::Error::other("the screen cannot be shown"));
    };
    let frame = viewing
        .screen
        .first_frame(FIRST_FRAME_TIMEOUT)
        .ok_or_else(|| io::Error::other("the screen showed no picture"))?;
    let area = frame.area();
    transport.write_all(&server_init(area.width, area.height, &host.name()))?;
    transport.flush()?;
    let mut viewer = Viewer {
        transport,
        viewing,
        encoder: Encoder::default(),
        request: None,
        sent: None,
        checked: 0,
        size: (area.width, area.height),
        unparsed: Vec::new(),
    };
    viewer.run()
}

/// Serves the viewers that connect to `listener` and that `allowed` lets through, each on a
/// thread of its own.
pub fn serve(
    listener: &TcpListener,
    host: Arc<dyn Host>,
    tls: Arc<ServerConfig>,
    allowed: impl Fn(IpAddr) -> bool,
) -> io::Result<()> {
    let attempts = Arc::new(Attempts::default());
    let viewers = Arc::new(AtomicUsize::new(0));
    loop {
        let (socket, address) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let peer = match address.ip() {
            IpAddr::V6(address) => address
                .to_ipv4_mapped()
                .map_or(IpAddr::V6(address), IpAddr::V4),
            peer => peer,
        };
        if !allowed(peer) {
            tracing::info!("refused a viewer at {peer}, which is not on the local network");
            continue;
        }
        if viewers.load(Ordering::Acquire) >= MOST_VIEWERS {
            tracing::info!("refused a viewer at {peer}: {MOST_VIEWERS} are connected");
            continue;
        }
        viewers.fetch_add(1, Ordering::AcqRel);
        let (host, tls, attempts, counted) = (
            Arc::clone(&host),
            Arc::clone(&tls),
            Arc::clone(&attempts),
            Arc::clone(&viewers),
        );
        let spawned = thread::Builder::new().name("viewer".into()).spawn(move || {
            match serve_viewer(socket, peer, host.as_ref(), &tls, &attempts) {
                Ok(()) => tracing::info!("the viewer at {peer} left"),
                Err(error) => tracing::info!("the viewer at {peer} left: {error}"),
            }
            counted.fetch_sub(1, Ordering::AcqRel);
        });
        if let Err(error) = spawned {
            viewers.fetch_sub(1, Ordering::AcqRel);
            tracing::warn!("cannot serve a viewer: {error}");
        }
    }
}
