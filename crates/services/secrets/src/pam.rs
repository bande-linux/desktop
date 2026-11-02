use std::io;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;

use async_io::Async;
use futures_lite::{AsyncReadExt, AsyncWriteExt};
use oo7::Secret;
use rustix::io::{FdFlags, fcntl_setfd};
use rustix::net::sockopt::socket_peercred;
use rustix::process::{getpid, getuid};
use zeroize::Zeroizing;

use crate::service::Daemon;

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

async fn answer(daemon: Arc<Daemon>, mut stream: Async<UnixStream>) -> io::Result<()> {
    let peer = socket_peercred(stream.as_fd())?;
    if !peer.uid.is_root() && peer.uid != getuid() {
        tracing::warn!("refused a login message from uid {}", peer.uid.as_raw());
        return Ok(());
    }
    let mut message = Zeroizing::new(Vec::new());
    (&mut stream)
        .take(LONGEST_MESSAGE as u64 + 1)
        .read_to_end(&mut message)
        .await?;
    let done = match parse(&message) {
        Some(Request::Unlock(password)) => daemon.login(password).await,
        Some(Request::Change { old, new }) => daemon.change_login(old, new).await,
        None => {
            tracing::warn!("ignored a malformed login message");
            false
        }
    };
    stream
        .write_all(&[if done { SUCCESS } else { FAILURE }])
        .await
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
