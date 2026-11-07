use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use bande_volumes::{Mounted, connected_roots, mounted};
use rustix::fs::statvfs;
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

use crate::mounts::{ConnectRequest, Mounts, Outcome};
use crate::server::Server;

const HEALTHY_EVERY: Duration = Duration::from_secs(10);
const TROUBLE_EVERY: Duration = Duration::from_secs(5);
const LOST_EVERY: Duration = Duration::from_secs(30);
const ANSWER_WITHIN: Duration = Duration::from_secs(6);
const CONNECT_WITHIN: Duration = Duration::from_secs(4);
const MISSES_BEFORE_ALERT: u32 = 2;
const DEAD_AFTER: Duration = Duration::from_secs(60);
const NM: &str = "org.freedesktop.NetworkManager";
const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
const NM_CONNECTED_LOCAL: u32 = 50;

#[derive(Default)]
struct Watched {
    misses: u32,
    alerted_since: Option<Instant>,
    alert: Option<u32>,
    stuck: Arc<Mutex<bool>>,
}

#[derive(Debug, Clone)]
pub struct Lost {
    pub request: ConnectRequest,
    pub name: String,
    pub alert: Option<u32>,
}

#[derive(Default)]
pub struct Shared {
    awake: Mutex<u64>,
    nudge: Condvar,
    pub unresponsive: Mutex<Vec<PathBuf>>,
    pub lost: Mutex<Vec<Lost>>,
}

fn guard<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn wake(&self) {
        *guard(&self.awake) += 1;
        self.nudge.notify_all();
    }

    fn wait(&self, seen: &mut u64, limit: Option<Duration>) {
        let awake = guard(&self.awake);
        let check = |awake: &mut u64| *awake == *seen;
        let awake = match limit {
            Some(limit) => {
                self.nudge
                    .wait_timeout_while(awake, limit, check)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0
            }
            None => self
                .nudge
                .wait_while(awake, check)
                .unwrap_or_else(PoisonError::into_inner),
        };
        *seen = *awake;
    }
}

fn server_of(mount: &Mounted) -> Option<Server> {
    let text = match mount.fs_type.as_str() {
        "cifs" | "smb3" => format!("smb:{}", mount.source),
        "nfs" | "nfs4" => {
            let (host, path) = mount.source.split_once(':')?;
            format!("nfs://{host}{path}")
        }
        "fuse.sshfs" => {
            let (host, path) = mount.source.split_once(':')?;
            format!("sftp://{host}/{path}")
        }
        "fuse" => mount.source.clone(),
        _ => return None,
    };
    Server::parse(&text)
}

fn connected_mounts() -> Vec<Mounted> {
    let roots = connected_roots();
    let table = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    mounted(&table)
        .filter(|mount| {
            mount
                .point
                .parent()
                .is_some_and(|parent| roots.iter().any(|root| root == parent))
        })
        .collect()
}

fn reachable(host: &str, port: u16) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (host, port).to_socket_addrs().is_ok_and(|addresses| {
        addresses
            .into_iter()
            .any(|address| TcpStream::connect_timeout(&address, CONNECT_WITHIN).is_ok())
    })
}

fn probe(point: &Path, server: &Server, stuck: &Arc<Mutex<bool>>) -> bool {
    if *guard(stuck) {
        return false;
    }
    *guard(stuck) = true;
    let (done, finished) = mpsc::channel();
    let (checked, host, port) = (point.to_owned(), server.host.clone(), server.port());
    let held = Arc::clone(stuck);
    let spawned = thread::Builder::new()
        .name("mount probe".to_owned())
        .spawn(move || {
            let answered = reachable(&host, port) && statvfs(&checked).is_ok();
            *guard(&held) = false;
            let _ = done.send(answered);
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot probe a mount: {error}");
        *guard(stuck) = false;
        return true;
    }
    finished.recv_timeout(ANSWER_WITHIN).unwrap_or(false)
}

pub fn follow(mounts: Mounts) {
    let spawned = thread::Builder::new()
        .name("mount health".to_owned())
        .spawn(move || watch(&mounts));
    if let Err(error) = spawned {
        tracing::warn!("cannot follow mounts: {error}");
    }
}

fn watch(mounts: &Mounts) {
    let shared = mounts.health();
    let mut watched: HashMap<PathBuf, Watched> = HashMap::new();
    let mut seen = 0;
    loop {
        let present = connected_mounts();
        watched.retain(|point, _| present.iter().any(|mount| mount.point == *point));
        let mut trouble = false;
        for mount in &present {
            let Some(server) = mounts.server_at(&mount.point).or_else(|| server_of(mount)) else {
                continue;
            };
            let entry = watched.entry(mount.point.clone()).or_default();
            match probe(&mount.point, &server, &entry.stuck) {
                true => recovered(mounts, &mount.point, entry),
                false => {
                    trouble = true;
                    missed(mounts, &mount.point, &server, entry);
                }
            }
        }
        let lost = !guard(&shared.lost).is_empty();
        if lost {
            mounts.try_lost();
        }
        let limit = match (trouble, lost, present.is_empty()) {
            (true, _, _) => Some(TROUBLE_EVERY),
            (false, _, false) => Some(HEALTHY_EVERY),
            (false, true, true) => Some(LOST_EVERY),
            (false, false, true) => None,
        };
        shared.wait(&mut seen, limit);
    }
}

fn recovered(mounts: &Mounts, point: &Path, entry: &mut Watched) {
    entry.misses = 0;
    entry.alerted_since = None;
    if let Some(id) = entry.alert.take() {
        mounts.close_alert(id);
    }
    mounts.set_unresponsive(point, false);
}

fn missed(mounts: &Mounts, point: &Path, server: &Server, entry: &mut Watched) {
    entry.misses += 1;
    if entry.misses < MISSES_BEFORE_ALERT {
        return;
    }
    let name = point
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let since = *entry.alerted_since.get_or_insert_with(Instant::now);
    if entry.alert.is_none() {
        entry.alert = mounts.alert_unresponsive(&name, point);
        mounts.set_unresponsive(point, true);
    }
    if since.elapsed() >= DEAD_AFTER {
        mounts.mark_lost(point, server, &name, entry.alert.take());
    }
}

fn follow_signal(
    rule: MatchRule<'static>,
    system: &Connection,
    nudged: impl Fn(&zbus::Message) -> bool + Send + 'static,
    shared: Arc<Shared>,
) {
    let changes = match MessageIterator::for_match_rule(rule, system, None) {
        Ok(changes) => changes,
        Err(error) => {
            tracing::warn!("cannot follow the system: {error}");
            return;
        }
    };
    let spawned = thread::Builder::new()
        .name("mount nudge".to_owned())
        .spawn(move || {
            for change in changes.flatten() {
                if nudged(&change) {
                    shared.wake();
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot follow the system: {error}");
    }
}

pub fn follow_network_and_wake(shared: &Arc<Shared>) {
    let Ok(system) = Connection::system() else {
        return;
    };
    let network = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(NM)
        .and_then(|rule| rule.interface(NM))
        .and_then(|rule| rule.member("StateChanged"))
        .map(|rule| rule.build());
    if let Ok(rule) = network {
        follow_signal(
            rule,
            &system,
            |change| {
                change
                    .body()
                    .deserialize::<(u32,)>()
                    .is_ok_and(|(state,)| state >= NM_CONNECTED_LOCAL)
            },
            Arc::clone(shared),
        );
    }
    let sleep = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(LOGIND)
        .and_then(|rule| rule.interface(LOGIND_MANAGER))
        .and_then(|rule| rule.member("PrepareForSleep"))
        .map(|rule| rule.build());
    if let Ok(rule) = sleep {
        follow_signal(
            rule,
            &system,
            |change| {
                change
                    .body()
                    .deserialize::<(bool,)>()
                    .is_ok_and(|(sleeping,)| !sleeping)
            },
            Arc::clone(shared),
        );
    }
}

pub fn should_retry(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Unreachable | Outcome::Failed(_))
}
