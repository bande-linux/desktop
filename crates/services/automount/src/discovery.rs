use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const RESOLVE: &str = "/run/systemd/resolve/io.systemd.Resolve";
const RETRIES: [u64; 3] = [2, 8, 30];
const SERVICES: [(&str, &str); 5] = [
    ("_nfs._tcp", "nfs"),
    ("_smb._tcp", "smb"),
    ("_sftp-ssh._tcp", "sftp"),
    ("_webdav._tcp", "dav"),
    ("_webdavs._tcp", "davs"),
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Nearby {
    pub name: String,
    pub url: String,
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
        list.sort();
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

fn browse(
    kind: &str,
    scheme: &str,
    found: &Mutex<Found>,
    changed: &(impl Fn() + Send + Sync),
) -> io::Result<()> {
    let mut stream = UnixStream::connect(RESOLVE)?;
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
            let flag = update.get("updateFlag").and_then(Value::as_str);
            let key = format!("{kind}/{name}");
            let mut entries = found.lock().unwrap_or_else(PoisonError::into_inner);
            let before = entries.list();
            match flag {
                Some("added") if name != own && !entries.entries.iter().any(|e| e.0 == key) => {
                    drop(entries);
                    let ifindex = update.get("ifindex").and_then(Value::as_i64).unwrap_or(0);
                    let Some((host, port)) = locate(name, kind, ifindex) else {
                        continue;
                    };
                    let entry = Nearby {
                        name: name.to_owned(),
                        url: url_of(scheme, &host, port),
                    };
                    entries = found.lock().unwrap_or_else(PoisonError::into_inner);
                    entries.entries.push((key, entry));
                }
                Some("removed") => entries.entries.retain(|entry| entry.0 != key),
                _ => {}
            }
            let after = entries.list();
            drop(entries);
            if before != after {
                changed();
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Discovery {
    found: Arc<Mutex<Found>>,
}

impl Discovery {
    pub fn start(changed: impl Fn() + Send + Sync + 'static) -> Self {
        let discovery = Self::default();
        let changed = Arc::new(changed);
        for (kind, scheme) in SERVICES {
            let found = Arc::clone(&discovery.found);
            let changed = Arc::clone(&changed);
            let spawned = thread::Builder::new()
                .name("network browser".to_owned())
                .spawn(move || {
                    for delay in RETRIES {
                        if let Err(error) = browse(kind, scheme, &found, &*changed) {
                            tracing::debug!("network browsing for {kind} stopped: {error}");
                        }
                        thread::sleep(Duration::from_secs(delay));
                    }
                });
            if let Err(error) = spawned {
                tracing::warn!("cannot browse the network: {error}");
            }
        }
        discovery
    }

    pub fn list(&self) -> Vec<Nearby> {
        self.found
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .list()
    }
}
