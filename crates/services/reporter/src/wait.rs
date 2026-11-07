use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::inotify;
use rustix::io::Errno;
use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::{AddressFamily, SocketFlags, SocketType, bind, socket_with};

const ROUTE_GROUPS: u32 = 0x1 | 0x10 | 0x40 | 0x100 | 0x400;
const SETTLE: Duration = Duration::from_secs(2);
const DRAIN_BYTES: usize = 8192;

#[derive(Debug, PartialEq, Eq)]
pub enum Woken {
    Deadline,
    Network,
    Config,
}

pub struct Waiter {
    network: Option<OwnedFd>,
    config: Option<OwnedFd>,
}

fn route_events() -> io::Result<OwnedFd> {
    let events = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        None,
    )?;
    bind(&events, &SocketAddrNetlink::new(0, ROUTE_GROUPS))?;
    Ok(events)
}

fn folder_changes(folder: &Path) -> io::Result<OwnedFd> {
    let changes = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
    inotify::add_watch(
        &changes,
        folder,
        inotify::WatchFlags::CREATE
            | inotify::WatchFlags::MOVED_TO
            | inotify::WatchFlags::CLOSE_WRITE
            | inotify::WatchFlags::DELETE,
    )?;
    Ok(changes)
}

fn drain(fd: &OwnedFd) {
    let mut buffer = [0; DRAIN_BYTES];
    while matches!(rustix::io::read(fd, &mut buffer), Ok(read) if read > 0) {}
}

fn timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(duration.subsec_nanos()),
    }
}

impl Waiter {
    pub fn new(config_file: &Path) -> Self {
        let network = route_events()
            .inspect_err(|error| tracing::warn!("cannot follow network changes: {error}"))
            .ok();
        let config = config_file
            .parent()
            .and_then(|folder| folder_changes(folder).ok());
        Self { network, config }
    }

    pub fn wait(&self, deadline: Option<Instant>, network_ends_wait: bool) -> io::Result<Woken> {
        loop {
            let mut fds = Vec::with_capacity(2);
            if let Some(config) = &self.config {
                fds.push(PollFd::new(config, PollFlags::IN));
            }
            if let Some(network) = &self.network {
                fds.push(PollFd::new(network, PollFlags::IN));
            }
            let left = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
            if left == Some(Duration::ZERO) {
                return Ok(Woken::Deadline);
            }
            match poll(&mut fds, left.map(timespec).as_ref()) {
                Ok(0) => return Ok(Woken::Deadline),
                Ok(_) => {}
                Err(Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
            let ready: Vec<bool> = fds
                .iter()
                .map(|fd| fd.revents().contains(PollFlags::IN))
                .collect();
            drop(fds);
            let mut ready = ready.into_iter();
            if self.config.is_some() && ready.next() == Some(true) {
                if let Some(config) = &self.config {
                    drain(config);
                }
                return Ok(Woken::Config);
            }
            if self.network.is_some()
                && ready.next() == Some(true)
                && let Some(network) = &self.network
            {
                self.settle(network);
                if network_ends_wait {
                    return Ok(Woken::Network);
                }
            }
        }
    }

    fn settle(&self, network: &OwnedFd) {
        std::thread::sleep(SETTLE);
        drain(network);
    }
}
