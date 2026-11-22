use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use bande_keyring::Keyring;
use bande_l10n::tr;
use bande_rfb::client::{self, Command, Credentials, Failure, Prompter, Screen};
use bande_rfb::encode::Frame;
use bande_rfb::protocol::Area;
use bande_toolkit::Region;
use rustix::fs::{MemfdFlags, memfd_create};

use crate::address::Target;

const KEYRING: &str = "org.bande.ScreenSharing";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(60);
const KNOWN: &str = "known-computers";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Question {
    Trust {
        fingerprint: String,
        changed: bool,
    },
    Unencrypted,
    Downgraded,
    Password {
        user: Option<String>,
        refused: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Signed(Credentials),
}

pub enum Event {
    Ask(Question),
    Connected {
        name: String,
        encrypted: bool,
    },
    Resized {
        memory: File,
        width: u32,
        height: u32,
    },
    Painted(Vec<Region>),
    Copied(String),
    Ended(Option<String>),
}

/// A connection to one computer, run on a thread of its own.
pub struct Connection {
    events: Receiver<Event>,
    ring: UnixStream,
    answers: Sender<Answer>,
    commander: Commander,
}

/// Sends what the person does to the other computer.
#[derive(Clone)]
pub struct Commander {
    commands: Sender<Command>,
    wake: Arc<UnixStream>,
}

impl Commander {
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_ok()
            && let Err(error) = (&*self.wake).write_all(&[1])
            && error.kind() != ErrorKind::WouldBlock
        {
            tracing::debug!("cannot wake the connection: {error}");
        }
    }
}

struct Feed {
    events: Sender<Event>,
    ring: UnixStream,
}

impl Feed {
    fn send(&self, event: Event) -> bool {
        let sent = self.events.send(event).is_ok();
        if sent
            && let Err(error) = (&self.ring).write_all(&[1])
            && error.kind() != ErrorKind::WouldBlock
        {
            tracing::debug!("cannot wake the viewer window: {error}");
        }
        sent
    }
}

fn known_file() -> Option<PathBuf> {
    bande_xdg::data_home().map(|data| data.join("bande/screen-sharing").join(KNOWN))
}

fn known_fingerprint(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(known_file()?).ok()?;
    text.lines().find_map(|line| {
        let (computer, fingerprint) = line.split_once(' ')?;
        (computer == key).then(|| fingerprint.to_owned())
    })
}

fn remember_fingerprint(key: &str, fingerprint: &str) {
    let Some(file) = known_file() else {
        return;
    };
    let changed = bande_xdg::change_file(&file, |text| {
        let mut lines: Vec<String> = text
            .lines()
            .filter(|line| {
                line.split_once(' ')
                    .is_none_or(|(computer, _)| computer != key)
            })
            .map(str::to_owned)
            .collect();
        lines.push(format!("{key} {fingerprint}"));
        Ok(lines.join("\n") + "\n")
    });
    if let Err(error) = changed {
        tracing::warn!("cannot remember the computer's certificate: {error}");
    }
}

fn login_name() -> String {
    std::env::var("USER").unwrap_or_default()
}

fn keyring() -> Option<Keyring> {
    Keyring::open(KEYRING)
        .inspect_err(|error| tracing::warn!("cannot open the keyring: {error}"))
        .ok()
}

struct Asking<'a> {
    target: &'a Target,
    feed: &'a Feed,
    answers: &'a Receiver<Answer>,
    saved: Option<String>,
    refused: Option<String>,
    entered: Option<String>,
    downgrade_allowed: bool,
}

impl Asking<'_> {
    fn ask(&self, question: Question) -> Option<Answer> {
        self.feed.send(Event::Ask(question));
        self.answers.recv().ok()
    }
}

impl Prompter for Asking<'_> {
    fn trust(&mut self, fingerprint: &str) -> bool {
        let key = self.target.key();
        let known = known_fingerprint(&key);
        if known.as_deref() == Some(fingerprint) {
            return true;
        }
        let trusted = self.ask(Question::Trust {
            fingerprint: fingerprint.to_owned(),
            changed: known.is_some(),
        }) == Some(Answer::Yes);
        if trusted {
            remember_fingerprint(&key, fingerprint);
            if known.is_some() {
                self.saved = None;
            }
        }
        trusted
    }

    fn unencrypted(&mut self) -> bool {
        self.downgrade_allowed || self.ask(Question::Unencrypted) == Some(Answer::Yes)
    }

    fn credentials(&mut self, user: bool, encrypted: bool) -> Option<Credentials> {
        if let Some(password) = self.saved.take().filter(|_| encrypted) {
            return Some(Credentials {
                user: login_name(),
                password,
            });
        }
        let question = Question::Password {
            user: user.then(login_name),
            refused: self.refused.take(),
        };
        match self.ask(question)? {
            Answer::Signed(credentials) => {
                self.entered = encrypted.then(|| credentials.password.clone());
                Some(credentials)
            }
            Answer::Yes | Answer::No => None,
        }
    }
}

struct Shown<'a> {
    feed: &'a Feed,
    memory: Option<File>,
}

fn region(area: &Area) -> Region {
    Region {
        x: u32::from(area.x),
        y: u32::from(area.y),
        width: u32::from(area.width),
        height: u32::from(area.height),
    }
}

impl Screen for Shown<'_> {
    fn resized(&mut self, frame: &Frame) {
        let memory = memfd_create("bande-screen-sharing", MemfdFlags::CLOEXEC)
            .map(File::from)
            .map_err(io::Error::from)
            .and_then(|memory| {
                memory.set_len(u64::from(frame.width) * u64::from(frame.height) * 4)?;
                Ok(memory)
            });
        let memory = match memory.and_then(|memory| Ok((memory.try_clone()?, memory))) {
            Ok(memory) => memory,
            Err(error) => {
                tracing::warn!("cannot make room for the screen: {error}");
                self.memory = None;
                return;
            }
        };
        self.memory = Some(memory.1);
        self.feed.send(Event::Resized {
            memory: memory.0,
            width: frame.width,
            height: frame.height,
        });
    }

    fn painted(&mut self, frame: &Frame, areas: &[Area]) {
        let Some(memory) = &self.memory else {
            return;
        };
        let stride = frame.width as usize;
        for area in areas {
            let (x, width) = (usize::from(area.x), usize::from(area.width));
            for y in usize::from(area.y)..usize::from(area.y) + usize::from(area.height) {
                let start = y * stride + x;
                let bytes: Vec<u8> = frame.pixels[start..start + width]
                    .iter()
                    .flat_map(|pixel| pixel.to_le_bytes())
                    .collect();
                if let Err(error) = memory.write_all_at(&bytes, (start * 4) as u64) {
                    tracing::warn!("cannot show the screen: {error}");
                    return;
                }
            }
        }
        self.feed
            .send(Event::Painted(areas.iter().map(region).collect()));
    }

    fn copied(&mut self, text: String) {
        self.feed.send(Event::Copied(text));
    }
}

fn reach(target: &Target) -> io::Result<TcpStream> {
    let mut last = io::Error::new(ErrorKind::NotFound, "no address");
    for address in (target.host.as_str(), target.port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(socket) => {
                socket.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
                socket.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
                socket.set_nodelay(true)?;
                return Ok(socket);
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

fn run(
    target: &Target,
    feed: &Feed,
    answers: &Receiver<Answer>,
    commands: &Receiver<Command>,
    wake: &UnixStream,
) -> Option<String> {
    let key = [("server", target.key())];
    let key: Vec<(&str, &str)> = key
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let mut saved = keyring().and_then(|keyring| {
        keyring
            .password(&key)
            .inspect_err(|error| tracing::warn!("cannot read a kept password: {error}"))
            .ok()
            .flatten()
    });
    let mut refused = None;
    let mut downgrade_allowed = false;
    loop {
        let socket = match reach(target) {
            Ok(socket) => socket,
            Err(error) => {
                return Some(tr!(
                    "Can’t reach {computer}: {reason}",
                    computer = target.host.clone(),
                    reason = error.to_string()
                ));
            }
        };
        let mut asking = Asking {
            target,
            feed,
            answers,
            saved: saved.take(),
            refused: refused.take(),
            entered: None,
            downgrade_allowed,
        };
        let pinned = !downgrade_allowed && known_fingerprint(&target.key()).is_some();
        let connected = client::connect(socket, &target.host, pinned, &mut asking);
        saved = asking.saved.take();
        match connected {
            Ok(session) => {
                if let (Some(password), Some(keyring)) = (asking.entered.take(), keyring()) {
                    let label = tr!(
                        "Screen Sharing password for {computer}",
                        computer = target.host.clone()
                    );
                    if let Err(error) = keyring.save(&key, &label, &password) {
                        tracing::warn!("cannot keep the password: {error}");
                    }
                }
                feed.send(Event::Connected {
                    name: session.name.clone(),
                    encrypted: session.transport.is_encrypted(),
                });
                let mut shown = Shown { feed, memory: None };
                return match client::view(session, commands, wake, &mut shown) {
                    Ok(()) => None,
                    Err(failure) => Some(ended(target, failure)),
                };
            }
            Err(Failure::Denied(reason)) => {
                refused = Some(match reason.is_empty() {
                    true => tr!("The password wasn’t accepted."),
                    false => reason,
                });
            }
            Err(Failure::Downgraded) => {
                feed.send(Event::Ask(Question::Downgraded));
                if answers.recv().ok() != Some(Answer::Yes) {
                    return None;
                }
                downgrade_allowed = true;
                saved = None;
            }
            Err(Failure::Cancelled) => return None,
            Err(failure) => return Some(ended(target, failure)),
        }
    }
}

fn ended(target: &Target, failure: Failure) -> String {
    match failure {
        Failure::Refused(reason) | Failure::Denied(reason) => reason,
        Failure::Unsupported => tr!(
            "{computer} shares its screen in a way this viewer doesn’t support.",
            computer = target.host.clone()
        ),
        Failure::Io(error) => tr!(
            "The connection to {computer} ended: {reason}",
            computer = target.host.clone(),
            reason = error.to_string()
        ),
        Failure::Cancelled => String::new(),
        Failure::Downgraded => tr!(
            "{computer} no longer offers encryption.",
            computer = target.host.clone()
        ),
    }
}

impl Connection {
    pub fn start(target: Target) -> io::Result<Self> {
        let (ring, rung) = UnixStream::pair()?;
        ring.set_nonblocking(true)?;
        rung.set_nonblocking(true)?;
        let (wake, woken) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        let (events, received) = mpsc::channel();
        let (answers, answered) = mpsc::channel();
        let (commands, ordered) = mpsc::channel();
        thread::Builder::new()
            .name("connection".into())
            .spawn(move || {
                let feed = Feed { events, ring: rung };
                let reason = run(&target, &feed, &answered, &ordered, &woken);
                feed.send(Event::Ended(reason.filter(|reason| !reason.is_empty())));
            })?;
        Ok(Self {
            events: received,
            ring,
            answers,
            commander: Commander {
                commands,
                wake: Arc::new(wake),
            },
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.ring.as_fd()
    }

    pub fn events(&self) -> Vec<Event> {
        let mut drained = [0; 256];
        while matches!((&self.ring).read(&mut drained), Ok(read) if read > 0) {}
        self.events.try_iter().collect()
    }

    pub fn answer(&self, answer: Answer) {
        let _ = self.answers.send(answer);
    }

    pub fn commander(&self) -> Commander {
        self.commander.clone()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.commander.wake.shutdown(Shutdown::Write);
    }
}
