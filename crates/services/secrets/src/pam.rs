use std::io;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_io::Async;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use oo7::Secret;
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::net::sockopt::socket_peercred;
use rustix::process::getpid;
use zeroize::Zeroizing;

use crate::service::Daemon;
use crate::store::{LOGIN, Store};

pub const SOCKET: &str = "bande-secrets/pam";
const UNLOCK: u8 = 1;
const CHANGE: u8 = 2;
const LONGEST_FIELD: usize = 4096;
const LONGEST_MESSAGE: usize = 1 + 2 * (4 + LONGEST_FIELD);
const FIRST_PASSED_FD: i32 = 3;
const SUCCESS: u8 = 0;
const FAILURE: u8 = 1;

pub enum Request {
    Unlock(Secret),
    Change { old: Secret, new: Secret },
}

fn activated() -> bool {
    let ours = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|pid| pid.parse::<i32>().ok())
        .is_some_and(|pid| pid == getpid().as_raw_nonzero().get());
    ours && std::env::var("LISTEN_FDS").is_ok_and(|count| count == "1")
}

pub fn listener(runtime_dir: &Path) -> io::Result<UnixListener> {
    if activated() {
        // SAFETY: LISTEN_PID and LISTEN_FDS say systemd passed this process exactly one socket, at
        // fd 3; main calls listener once at startup, so nothing else owns that descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(FIRST_PASSED_FD) };
        fcntl_setfd(&fd, FdFlags::CLOEXEC)?;
        return Ok(UnixListener::from(fd));
    }
    let path = runtime_dir.join(SOCKET);
    if let Some(directory) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn field<'a>(bytes: &mut &'a [u8]) -> Option<&'a [u8]> {
    let (length, rest) = bytes.split_first_chunk::<4>()?;
    let length = usize::try_from(u32::from_le_bytes(*length)).ok()?;
    if length > LONGEST_FIELD || rest.len() < length {
        return None;
    }
    let (value, rest) = rest.split_at(length);
    *bytes = rest;
    Some(value)
}

pub fn parse(message: &[u8]) -> Option<Request> {
    let (operation, mut rest) = message.split_first()?;
    let request = match *operation {
        UNLOCK => Request::Unlock(Secret::blob(field(&mut rest)?)),
        CHANGE => {
            let old = Secret::blob(field(&mut rest)?);
            let new = Secret::blob(field(&mut rest)?);
            Request::Change { old, new }
        }
        _ => return None,
    };
    rest.is_empty().then_some(request)
}

fn from_root(stream: &Async<UnixStream>) -> io::Result<bool> {
    let peer = socket_peercred(stream.as_fd())?;
    if !peer.uid.is_root() {
        tracing::warn!("refused a login message from uid {}", peer.uid.as_raw());
    }
    Ok(peer.uid.is_root())
}

async fn request(stream: &mut Async<UnixStream>) -> io::Result<Option<Request>> {
    let mut message = Zeroizing::new(Vec::new());
    stream
        .take(LONGEST_MESSAGE as u64 + 1)
        .read_to_end(&mut message)
        .await?;
    let request = parse(&message);
    if request.is_none() {
        tracing::warn!("ignored a malformed login message");
    }
    Ok(request)
}

async fn reply(stream: &mut Async<UnixStream>, done: bool) -> io::Result<()> {
    stream
        .write_all(&[if done { SUCCESS } else { FAILURE }])
        .await
}

async fn answer(daemon: Arc<Daemon>, mut stream: Async<UnixStream>) -> io::Result<()> {
    if !from_root(&stream)? {
        return Ok(());
    }
    let done = match request(&mut stream).await? {
        Some(Request::Unlock(password)) => daemon.login(password).await,
        Some(Request::Change { old, new }) => daemon.change_login(old, new).await,
        None => false,
    };
    reply(&mut stream, done).await
}

async fn change_outside_bande(data_home: PathBuf, old: Secret, new: Secret) -> bool {
    let mut store = Store::load(data_home).await;
    if !store.exists(LOGIN) {
        return true;
    }
    if let Err(error) = store.unlock(LOGIN, old).await {
        if store.unlock(LOGIN, new).await.is_ok() {
            return true;
        }
        tracing::info!("kept the login keyring password: {error}");
        return false;
    }
    match store.change_password(LOGIN, new).await {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!("cannot change the login keyring password: {error}");
            false
        }
    }
}

/// Answers the login messages already waiting when this user has no Bande session: the login
/// keyring follows a password change, and nothing is unlocked.
pub async fn follow_changes(data_home: PathBuf, listener: UnixListener) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error),
        };
        if let Err(error) = answer_outside_bande(data_home.clone(), stream).await {
            tracing::warn!("login message: {error}");
        }
    }
}

async fn answer_outside_bande(data_home: PathBuf, stream: UnixStream) -> io::Result<()> {
    let mut stream = Async::new(stream)?;
    if !from_root(&stream)? {
        return Ok(());
    }
    let done = match request(&mut stream).await? {
        Some(Request::Change { old, new }) => change_outside_bande(data_home, old, new).await,
        Some(Request::Unlock(_)) => {
            tracing::info!("left the keyring locked outside a Bande session");
            false
        }
        None => false,
    };
    reply(&mut stream, done).await
}

pub async fn serve(
    daemon: Arc<Daemon>,
    connection: zbus::Connection,
    listener: UnixListener,
) -> io::Result<()> {
    let listener = Async::new(listener)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let daemon = daemon.clone();
        connection
            .executor()
            .spawn(
                async move {
                    if let Err(error) = answer(daemon, stream).await {
                        tracing::warn!("login message: {error}");
                    }
                },
                "bande-secrets login",
            )
            .detach();
    }
}
