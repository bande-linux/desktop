use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::inotify;
use rustix::io::Errno;
use rustix::net::netlink::{self, SocketAddrNetlink};
use rustix::net::{AddressFamily, Protocol, SocketFlags, SocketType, bind, socket_with};

const ROUTE_GROUPS: u32 = 0x1 | 0x10 | 0x40 | 0x100 | 0x400;
const KERNEL_UEVENTS: u32 = 0x1;
const POWER_SUPPLY: &[u8] = b"SUBSYSTEM=power_supply";
const SETTLE: Duration = Duration::from_secs(2);
const DRAIN_BYTES: usize = 8192;

#[derive(Debug, PartialEq, Eq)]
pub enum Woken {
    Deadline,
    Network,
    Config,
    Outbox,
    Power,
}

#[derive(Clone, Copy, Default)]
pub struct Wakes {
    pub network: bool,
    pub power: bool,
    pub outbox: bool,
}

pub struct Waiter {
    network: Option<OwnedFd>,
    config: Option<OwnedFd>,
    outbox: Option<OwnedFd>,
    power: Option<OwnedFd>,
}

fn netlink_socket(protocol: Option<Protocol>, groups: u32) -> io::Result<OwnedFd> {
    let events = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
        protocol,
    )?;
    bind(&events, &SocketAddrNetlink::new(0, groups))?;
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

fn drain(fd: &OwnedFd) -> bool {
    let mut buffer = [0; DRAIN_BYTES];
    let mut power = false;
    while let Ok(read) = rustix::io::read(fd, &mut buffer) {
        if read == 0 {
            break;
        }
        power |= buffer[..read]
            .windows(POWER_SUPPLY.len())
            .any(|window| window == POWER_SUPPLY);
    }
    power
}

fn timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(duration.subsec_nanos()),
    }
}

impl Waiter {
    pub fn new(config_file: &Path, outbox: &Path) -> Self {
        let network = netlink_socket(None, ROUTE_GROUPS)
            .inspect_err(|error| tracing::warn!("cannot follow network changes: {error}"))
            .ok();
        let config = config_file
            .parent()
            .and_then(|folder| folder_changes(folder).ok());
        Self {
            network,
            config,
            outbox: folder_changes(outbox)
                .inspect_err(|error| tracing::warn!("cannot follow the outbox: {error}"))
                .ok(),
            power: None,
        }
    }

    pub fn wait(&mut self, deadline: Option<Instant>, wakes: Wakes) -> io::Result<Woken> {
        if wakes.power && self.power.is_none() {
            self.power = netlink_socket(Some(netlink::KOBJECT_UEVENT), KERNEL_UEVENTS)
                .inspect_err(|error| tracing::warn!("cannot follow power changes: {error}"))
                .ok();
        }
        if let Some(power) = &self.power {
            drain(power);
        }
        loop {
            let sources: Vec<(&OwnedFd, Woken)> = [
                (self.config.as_ref(), Woken::Config),
                (self.outbox.as_ref().filter(|_| wakes.outbox), Woken::Outbox),
                (self.network.as_ref(), Woken::Network),
                (self.power.as_ref().filter(|_| wakes.power), Woken::Power),
            ]
            .into_iter()
            .filter_map(|(fd, woken)| Some((fd?, woken)))
            .collect();
            let mut fds: Vec<PollFd> = sources
                .iter()
                .map(|(fd, _)| PollFd::new(*fd, PollFlags::IN))
                .collect();
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
            for ((fd, woken), ready) in sources.into_iter().zip(ready) {
                if !ready {
                    continue;
                }
                match woken {
                    Woken::Config | Woken::Outbox => {
                        drain(fd);
                        return Ok(woken);
                    }
                    Woken::Network => {
                        std::thread::sleep(SETTLE);
                        drain(fd);
                        if wakes.network {
                            return Ok(Woken::Network);
                        }
                    }
                    Woken::Power => {
                        if drain(fd) {
                            return Ok(Woken::Power);
                        }
                    }
                    Woken::Deadline => {}
                }
            }
        }
    }
}
