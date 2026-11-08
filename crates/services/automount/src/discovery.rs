use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::names::BusName;

const RESOLVE: &str = "/run/systemd/resolve/io.systemd.Resolve";
const RETRIES: [u64; 3] = [2, 8, 30];
const SERVICES: [(&str, &str); 5] = [
    ("_smb._tcp", "smb"),
    ("_nfs._tcp", "nfs"),
    ("_sftp-ssh._tcp", "sftp"),
    ("_webdav._tcp", "dav"),
    ("_webdavs._tcp", "davs"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nearby {
    pub name: String,
    pub url: String,
}

fn preference(url: &str) -> usize {
    SERVICES
        .iter()
        .position(|(_, scheme)| url.starts_with(&format!("{scheme}://")))
        .unwrap_or(SERVICES.len())
}

#[derive(Default)]
struct Found {
    entries: Vec<(String, Nearby)>,
}

impl Found {
    fn list(&self) -> Vec<Nearby> {
        let mut list: Vec<Nearby> = self
            .entries
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect();
        list.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| preference(&a.url).cmp(&preference(&b.url)))
        });
        list.dedup_by(|a, b| a.name == b.name);
        list
    }
}

fn read_replies(
    stream: &mut UnixStream,
    pending: &mut Vec<u8>,
    mut each: impl FnMut(&Value),
) -> io::Result<()> {
    let mut buffer = [0u8; 4096];
    let read = stream.read(&mut buffer)?;
    if read == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    pending.extend_from_slice(&buffer[..read]);
    while let Some(end) = pending.iter().position(|byte| *byte == 0) {
        let message: Vec<u8> = pending.drain(..=end).collect();
        if let Ok(value) = serde_json::from_slice::<Value>(&message[..end]) {
            each(&value);
        }
    }
    Ok(())
}

fn send(stream: &mut UnixStream, request: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(request).map_err(io::Error::other)?;
    bytes.push(0);
    stream.write_all(&bytes)
}

fn hostname_and_port(reply: &Value) -> Option<(String, u64)> {
    let parameters = reply.get("parameters")?;
    let target = parameters.get("services")?.get(0)?;
    let host = target.get("hostname").and_then(Value::as_str)?;
    let port = target.get("port").and_then(Value::as_u64)?;
    let path = parameters
        .get("txt")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .find_map(|record| record.strip_prefix("path="))
        .unwrap_or_default();
    Some((format!("{}{path}", host.trim_end_matches('.')), port))
}

fn locate(name: &str, kind: &str, ifindex: i64) -> Option<(String, u64)> {
    let mut stream = UnixStream::connect(RESOLVE).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    send(
        &mut stream,
        &json!({
            "method": "io.systemd.Resolve.ResolveService",
            "parameters": {"name": name, "type": kind, "domain": "local", "ifindex": ifindex},
        }),
    )
    .ok()?;
    let mut pending = Vec::new();
    let mut answer = None;
    while answer.is_none() {
        read_replies(&mut stream, &mut pending, |reply| {
            answer = Some(hostname_and_port(reply));
        })
        .ok()?;
    }
    answer.flatten()
}

fn url_of(scheme: &str, host: &str, port: u64) -> String {
    let default = matches!(
        (scheme, port),
        ("smb", 445) | ("sftp", 22) | ("dav", 80) | ("davs", 443) | ("nfs", 2049)
    );
    match default {
        true => format!("{scheme}://{host}"),
        false => match host.split_once('/') {
            Some((name, path)) => format!("{scheme}://{name}:{port}/{path}"),
            None => format!("{scheme}://{host}:{port}"),
        },
    }
}

fn own_name() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn browse(kind: &str, scheme: &str, discovery: &Discovery, generation: u64) -> io::Result<()> {
    let mut stream = UnixStream::connect(RESOLVE)?;
    if !discovery.register(generation, &stream) {
        return Ok(());
    }
    send(
        &mut stream,
        &json!({
            "method": "io.systemd.Resolve.BrowseServices",
            "parameters": {"domain": "local", "type": kind, "ifindex": 0, "flags": 0},
            "more": true,
        }),
    )?;
    let own = own_name();
    let mut pending = Vec::new();
    loop {
        let mut updates = Vec::new();
        read_replies(&mut stream, &mut pending, |reply| {
            let list = reply
                .get("parameters")
                .and_then(|parameters| parameters.get("browserServiceData"))
                .and_then(Value::as_array);
            updates.extend(list.into_iter().flatten().cloned());
        })?;
        for update in updates {
            let Some(name) = update.get("name").and_then(Value::as_str) else {
                continue;
            };
            let key = format!("{kind}/{name}");
            match update.get("updateFlag").and_then(Value::as_str) {
                Some("added") if name != own && !discovery.knows(generation, &key) => {
                    let ifindex = update.get("ifindex").and_then(Value::as_i64).unwrap_or(0);
                    if let Some((host, port)) = locate(name, kind, ifindex) {
                        let entry = Nearby {
                            name: name.to_owned(),
                            url: url_of(scheme, &host, port),
                        };
                        discovery.update(generation, |found| found.entries.push((key, entry)));
                    }
                }
                Some("removed") => {
                    discovery.update(generation, |found| {
                        found.entries.retain(|entry| entry.0 != key);
                    });
                }
                _ => {}
            }
        }
    }
}

#[derive(Default)]
struct State {
    found: Found,
    watchers: HashSet<String>,
    generation: u64,
    streams: Vec<UnixStream>,
}

#[derive(Clone)]
pub struct Discovery {
    state: Arc<Mutex<State>>,
    session: Connection,
    changed: Arc<dyn Fn() + Send + Sync>,
}

impl Discovery {
    pub fn new(session: Connection, changed: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::default(),
            session,
            changed: Arc::new(changed),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn watch(&self, watcher: String) {
        let mut state = self.lock();
        if !state.watchers.insert(watcher.clone()) {
            return;
        }
        let first = state.watchers.len() == 1;
        let generation = state.generation;
        drop(state);
        self.follow_watcher(watcher);
        if first {
            self.start(generation);
        }
    }

    fn follow_watcher(&self, watcher: String) {
        let discovery = self.clone();
        let spawned = thread::Builder::new()
            .name("network watcher".to_owned())
            .spawn(move || {
                if let Err(error) = discovery.wait_until_gone(&watcher) {
                    tracing::warn!("cannot follow {watcher}: {error}");
                }
                discovery.unwatch(&watcher);
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot follow a network watcher: {error}");
            self.lock().watchers.clear();
        }
    }

    fn wait_until_gone(&self, watcher: &str) -> zbus::Result<()> {
        let bus = DBusProxy::new(&self.session)?;
        let changes = bus.receive_name_owner_changed_with_args(&[(0, watcher)])?;
        let name = BusName::try_from(watcher)?;
        if !bus.name_has_owner(name)? {
            return Ok(());
        }
        for change in changes {
            if change.args().is_ok_and(|args| args.new_owner().is_none()) {
                break;
            }
        }
        Ok(())
    }

    fn unwatch(&self, watcher: &str) {
        let mut state = self.lock();
        if !state.watchers.remove(watcher) || !state.watchers.is_empty() {
            return;
        }
        state.generation += 1;
        for stream in state.streams.drain(..) {
            if let Err(error) = stream.shutdown(Shutdown::Both) {
                tracing::debug!("network browsing already stopped: {error}");
            }
        }
        let had = !state.found.entries.is_empty();
        state.found.entries.clear();
        drop(state);
        if had {
            (self.changed)();
        }
    }

    fn start(&self, generation: u64) {
        for (kind, scheme) in SERVICES {
            let discovery = self.clone();
            let spawned = thread::Builder::new()
                .name("network browser".to_owned())
                .spawn(move || {
                    for delay in RETRIES {
                        if let Err(error) = browse(kind, scheme, &discovery, generation) {
                            tracing::debug!("network browsing for {kind} stopped: {error}");
                        }
                        if !discovery.current(generation) {
                            return;
                        }
                        thread::sleep(Duration::from_secs(delay));
                    }
                });
            if let Err(error) = spawned {
                tracing::warn!("cannot browse the network: {error}");
            }
        }
    }

    fn current(&self, generation: u64) -> bool {
        self.lock().generation == generation
    }

    fn register(&self, generation: u64, stream: &UnixStream) -> bool {
        let mut state = self.lock();
        if state.generation != generation {
            return false;
        }
        match stream.try_clone() {
            Ok(clone) => {
                state.streams.push(clone);
                true
            }
            Err(error) => {
                tracing::warn!("cannot browse the network: {error}");
                false
            }
        }
    }

    fn knows(&self, generation: u64, key: &str) -> bool {
        let state = self.lock();
        state.generation != generation || state.found.entries.iter().any(|entry| entry.0 == key)
    }

    fn update(&self, generation: u64, change: impl FnOnce(&mut Found)) {
        let mut state = self.lock();
        if state.generation != generation {
            return;
        }
        let before = state.found.list();
        change(&mut state.found);
        let changed = state.found.list() != before;
        drop(state);
        if changed {
            (self.changed)();
        }
    }

    pub fn list(&self) -> Vec<Nearby> {
        self.lock().found.list()
    }
}
