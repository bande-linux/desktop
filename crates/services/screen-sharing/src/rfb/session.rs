use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use bande_rfb::auth::{CHALLENGE, same, vnc_response};
use bande_rfb::clipboard::Clipboard;
use bande_rfb::encode::{Encoder, Frame, changed_areas};
use bande_rfb::protocol::{
    Area, ClientMessage, ENCODING_DESKTOP_SIZE, ENCODING_EXTENDED_CLIPBOARD, MOST_CUT_TEXT,
    SECURITY_VENCRYPT, SECURITY_VNC, VENCRYPT_X509_PLAIN, VENCRYPT_X509_VNC, VERSION, Version,
    cut_text_message, parse_message, parse_version, rectangle_header, server_init, update_header,
};
use bande_rfb::transport::{Transport, read_array, read_u32};
use ring::rand::{SecureRandom, SystemRandom};
use rustix::event::{PollFd, PollFlags, poll};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

use super::limits::{Limits, Seat};
use super::{Host, Viewing};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(60);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const FAILURE_DELAY: Duration = Duration::from_secs(1);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);
const MOST_CREDENTIAL: usize = 1024;
const MOST_UNPARSED: usize = MOST_CUT_TEXT + 64 * 1024;
const SERVER_CUT_TEXT: u8 = 3;
const BUTTONS: u8 = 3;
const WHEEL: [(u8, i32, i32); 4] = [
    (1 << 3, 0, -1),
    (1 << 4, 0, 1),
    (1 << 5, -1, 0),
    (1 << 6, 1, 0),
];
const READ_CHUNK: usize = 64 * 1024;
const FAILED: u32 = 1;

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
    older_viewers: bool,
) -> io::Result<(Transport, Checked)> {
    socket.write_all(&[0, 2])?;
    let version = read_array::<2>(&mut socket)?;
    if version != [0, 2] {
        socket.write_all(&[1])?;
        return Err(refused("the viewer speaks another VeNCrypt version"));
    }
    let offered: &[u32] = match older_viewers {
        true => &[VENCRYPT_X509_PLAIN, VENCRYPT_X509_VNC],
        false => &[VENCRYPT_X509_PLAIN],
    };
    socket.write_all(&[0, offered.len() as u8])?;
    for subtype in offered {
        socket.write_all(&subtype.to_be_bytes())?;
    }
    let chosen = read_u32(&mut socket)?;
    if !offered.contains(&chosen) {
        socket.write_all(&[0])?;
        return Err(refused("the viewer chose no offered VeNCrypt type"));
    }
    socket.write_all(&[1])?;
    let connection = ServerConnection::new(Arc::clone(tls)).map_err(io::Error::other)?;
    let mut stream = StreamOwned::new(connection, socket);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    let mut transport = Transport::Server(Box::new(stream));
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

fn shake_hands(
    mut socket: TcpStream,
    peer: IpAddr,
    host: &dyn Host,
    tls: &Arc<ServerConfig>,
    limits: &Arc<Limits>,
) -> io::Result<Option<Transport>> {
    socket.write_all(VERSION)?;
    let version = parse_version(&read_array::<12>(&mut socket)?)
        .ok_or_else(|| refused("the viewer speaks no known RFB version"))?;
    let password = host.password();
    let older_viewers = host.older_viewers();
    let refusal = match (&password, limits.blocked(peer)) {
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
                SECURITY_VENCRYPT => vencrypt(socket, tls, &password, older_viewers)?,
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
        limits.failed(peer);
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

struct Viewer<'a> {
    transport: Transport,
    viewing: Viewing,
    host: &'a dyn Host,
    clipboard: Clipboard,
    announced: bool,
    copy_serial: u64,
    last_text: Option<String>,
    encoder: Encoder,
    request: Option<Request>,
    sent: Option<Arc<Frame>>,
    checked: u64,
    size: (u16, u16),
    unparsed: Vec<u8>,
    buttons: u8,
    keys: Vec<u32>,
    _seat: Seat,
}

impl Drop for Viewer<'_> {
    fn drop(&mut self) {
        for keysym in std::mem::take(&mut self.keys) {
            self.viewing.input.key(keysym, false);
        }
        for button in 0..BUTTONS {
            if self.buttons & (1 << button) != 0 {
                self.viewing.input.button(button, false);
            }
        }
    }
}

impl Viewer<'_> {
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
        let mut encoded = 0;
        for area in &areas {
            encoded += usize::from(self.encoder.encode(frame, *area, &mut body)?);
        }
        self.sent = Some(Arc::clone(frame));
        let count = u16::try_from(rectangles.len() + encoded).unwrap_or(u16::MAX);
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
            ClientMessage::SetEncodings(encodings) => {
                self.encoder.set_encodings(&encodings);
                if encodings.contains(&ENCODING_EXTENDED_CLIPBOARD) && !self.announced {
                    self.announced = true;
                    let announce = self.clipboard.announce();
                    self.send_cut_text(&announce)?;
                }
            }
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
            ClientMessage::Key { down, keysym } => {
                let held = self.keys.iter().position(|key| *key == keysym);
                match (down, held) {
                    (true, None) => self.keys.push(keysym),
                    (false, Some(at)) => {
                        self.keys.remove(at);
                    }
                    _ => {}
                }
                self.viewing.input.key(keysym, down);
            }
            ClientMessage::Pointer { buttons, x, y } => self.point(buttons, x, y),
            ClientMessage::CutText(text) => {
                let outcome = self.clipboard.received(text);
                if let Some(reply) = outcome.reply {
                    self.send_cut_text(&reply)?;
                }
                if let Some(text) = outcome.copied.filter(|text| !text.is_empty()) {
                    self.last_text = Some(text.clone());
                    self.host.copied(text);
                }
            }
        }
        Ok(())
    }

    fn point(&mut self, buttons: u8, x: u16, y: u16) {
        let input = &self.viewing.input;
        input.pointer(u32::from(x), u32::from(y));
        let pressed = buttons & !self.buttons;
        let released = self.buttons & !buttons;
        for button in 0..BUTTONS {
            let bit = 1 << button;
            if pressed & bit != 0 || released & bit != 0 {
                input.button(button, pressed & bit != 0);
            }
        }
        for (bit, x, y) in WHEEL {
            if pressed & bit != 0 {
                input.scroll(x, y);
            }
        }
        self.buttons = buttons;
    }

    fn send_cut_text(&mut self, text: &bande_rfb::protocol::CutText) -> io::Result<()> {
        self.transport
            .write_all(&cut_text_message(SERVER_CUT_TEXT, text))?;
        self.transport.flush()
    }

    fn share_clipboard(&mut self, serial: u64, text: Option<String>) -> io::Result<()> {
        if serial == self.copy_serial {
            return Ok(());
        }
        self.copy_serial = serial;
        let Some(text) = text.filter(|text| self.last_text.as_ref() != Some(text)) else {
            return Ok(());
        };
        self.last_text = Some(text.clone());
        let message = self.clipboard.copied_here(text);
        self.send_cut_text(&message)
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
            self.share_clipboard(latest.copy_serial, latest.copied)?;
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
    limits: &Arc<Limits>,
    handshake: super::limits::Handshake,
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
    let shaken = shake_hands(socket, peer, host, tls, limits);
    drop(finished);
    drop(handshake);
    let Some(mut transport) = shaken? else {
        return Ok(());
    };
    let Some(seat) = limits.seat() else {
        return Err(io::Error::other("four viewers are connected"));
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
    let copy_serial = viewing.screen.latest().copy_serial;
    let mut viewer = Viewer {
        transport,
        viewing,
        host,
        clipboard: Clipboard::default(),
        announced: false,
        copy_serial,
        last_text: None,
        encoder: Encoder::default(),
        request: None,
        sent: None,
        checked: 0,
        size: (area.width, area.height),
        unparsed: Vec::new(),
        buttons: 0,
        keys: Vec::new(),
        _seat: seat,
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
    let limits = Arc::new(Limits::default());
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
        let Some(handshake) = limits.handshake(peer) else {
            tracing::info!("refused a viewer at {peer}: it is already connecting");
            continue;
        };
        let (host, tls, limits) = (Arc::clone(&host), Arc::clone(&tls), Arc::clone(&limits));
        let spawned =
            thread::Builder::new().name("viewer".into()).spawn(move || {
                match serve_viewer(socket, peer, host.as_ref(), &tls, &limits, handshake) {
                    Ok(()) => tracing::info!("the viewer at {peer} left"),
                    Err(error) => tracing::info!("the viewer at {peer} left: {error}"),
                }
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot serve a viewer: {error}");
        }
    }
}
