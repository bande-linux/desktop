//! Service discovery from the DNS records of the user's own domain: SRV (RFC 2782) and TXT
//! lookups for RFC 6186 (mail) and RFC 6764 (CalDAV, CardDAV), through the nameservers in
//! `/etc/resolv.conf`.

use std::hash::{BuildHasher, RandomState};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
use std::time::Duration;

#[cfg(feature = "testing")]
pub mod testing;

const RESOLV_CONF: &str = "/etc/resolv.conf";
const DNS_PORT: u16 = 53;
const TIMEOUT: Duration = Duration::from_secs(3);
const MAX_SERVERS: usize = 3;
const MAX_POINTERS: usize = 32;
const UDP_LIMIT: usize = 4096;
const SRV: u16 = 33;
const TXT: u16 = 16;
const TRUNCATED: u8 = 0x02;

/// One SRV target, in the order a client should try it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub host: String,
    pub port: u16,
}

/// The nameservers to ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolver {
    servers: Vec<SocketAddr>,
}

impl Default for Resolver {
    fn default() -> Self {
        Self::system()
    }
}

/// The domain of a mail address such as `ann@example.org`, in lower case, if it looks like one.
pub fn domain_of(address: &str) -> Option<String> {
    let (local, domain) = address.trim().rsplit_once('@')?;
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    let valid = !local.is_empty()
        && domain.contains('.')
        && domain.split('.').all(|label| {
            (1..64).contains(&label.len())
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    valid.then_some(domain)
}

impl Resolver {
    /// The nameservers of `/etc/resolv.conf`.
    pub fn system() -> Self {
        let text = std::fs::read_to_string(RESOLV_CONF)
            .inspect_err(|error| tracing::warn!("cannot read {RESOLV_CONF}: {error}"))
            .unwrap_or_default();
        Self::new(nameservers(&text))
    }

    /// Asks these nameservers instead, each at its own port.
    pub fn new(servers: Vec<SocketAddr>) -> Self {
        Self { servers }
    }

    /// The SRV targets of `name` (such as `_imaps._tcp.example.org`) by priority, then weight.
    /// Empty when there is no record or when the domain says the service is not offered.
    pub fn services(&self, name: &str) -> Vec<Service> {
        let mut records = self.resolve(name, SRV, |packet, at, _| {
            Some((
                read_u16(packet, at)?,
                read_u16(packet, at + 2)?,
                Service {
                    host: read_name(packet, at + 6)?.0.to_ascii_lowercase(),
                    port: read_u16(packet, at + 4)?,
                },
            ))
        });
        records.sort_by_key(|(priority, weight, _)| (*priority, u16::MAX - weight));
        records
            .into_iter()
            .map(|(_, _, service)| service)
            .filter(|service| !service.host.is_empty() && service.port != 0)
            .collect()
    }

    /// The character strings of every TXT record of `name`, such as `path=/dav/`.
    pub fn texts(&self, name: &str) -> Vec<String> {
        self.resolve(name, TXT, |packet, mut at, end| {
            let mut texts = Vec::new();
            while at < end {
                let length = usize::from(*packet.get(at)?);
                let text = packet.get(at + 1..at + 1 + length)?;
                texts.push(String::from_utf8_lossy(text).into_owned());
                at += 1 + length;
            }
            Some(texts)
        })
        .concat()
    }

    fn resolve<T>(&self, name: &str, kind: u16, parse: Parse<T>) -> Vec<T> {
        let id = RandomState::new().hash_one(name) as u16;
        let Some(query) = query(id, name, kind) else {
            return Vec::new();
        };
        for server in self.servers.iter().take(MAX_SERVERS) {
            match ask(*server, &query, id, kind, parse) {
                Ok(Some(records)) => return records,
                Ok(None) => {}
                Err(error) => tracing::debug!("{server} did not answer for {name}: {error}"),
            }
        }
        Vec::new()
    }
}

fn nameservers(resolv_conf: &str) -> Vec<SocketAddr> {
    resolv_conf
        .lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(|address| address.trim().split('%').next()?.parse::<IpAddr>().ok())
        .map(|address| SocketAddr::new(address, DNS_PORT))
        .collect()
}

fn query(id: u16, name: &str, kind: u16) -> Option<Vec<u8>> {
    let mut packet = Vec::with_capacity(32 + name.len());
    packet.extend(id.to_be_bytes());
    packet.extend([0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in name.trim_end_matches('.').split('.') {
        let length = u8::try_from(label.len())
            .ok()
            .filter(|length| (1..64).contains(length))?;
        packet.push(length);
        packet.extend(label.as_bytes());
    }
    packet.push(0);
    packet.extend(kind.to_be_bytes());
    packet.extend(1u16.to_be_bytes());
    Some(packet)
}

type Parse<T> = fn(&[u8], usize, usize) -> Option<T>;

fn ask<T>(
    server: SocketAddr,
    query: &[u8],
    id: u16,
    kind: u16,
    parse: Parse<T>,
) -> std::io::Result<Option<Vec<T>>> {
    let local: SocketAddr = match server.ip() {
        IpAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
        IpAddr::V6(_) => ([0u16; 8], 0).into(),
    };
    let socket = UdpSocket::bind(local)?;
    socket.set_read_timeout(Some(TIMEOUT))?;
    socket.send_to(query, server)?;
    let mut answer = vec![0u8; UDP_LIMIT];
    loop {
        let (length, from) = socket.recv_from(&mut answer)?;
        if from != server {
            continue;
        }
        let packet = &answer[..length];
        if packet.get(2).is_some_and(|flags| flags & TRUNCATED != 0) {
            return over_tcp(server, query, id, kind, parse);
        }
        if let Some(records) = records(packet, id, kind, parse) {
            return Ok(Some(records));
        }
    }
}

fn over_tcp<T>(
    server: SocketAddr,
    query: &[u8],
    id: u16,
    kind: u16,
    parse: Parse<T>,
) -> std::io::Result<Option<Vec<T>>> {
    let mut stream = TcpStream::connect_timeout(&server, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    let length = u16::try_from(query.len()).map_err(std::io::Error::other)?;
    stream.write_all(&[&length.to_be_bytes()[..], query].concat())?;
    let mut length = [0u8; 2];
    stream.read_exact(&mut length)?;
    let mut packet = vec![0u8; usize::from(u16::from_be_bytes(length))];
    stream.read_exact(&mut packet)?;
    Ok(records(&packet, id, kind, parse))
}

fn read_u16(packet: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*packet.get(at)?, *packet.get(at + 1)?]))
}

fn read_name(packet: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels = Vec::new();
    let mut end = None;
    for _ in 0..MAX_POINTERS {
        let length = *packet.get(at)?;
        match length {
            0 => return Some((labels.join("."), end.unwrap_or(at + 1))),
            pointer if pointer & 0xc0 == 0xc0 => {
                end.get_or_insert(at + 2);
                at = usize::from(read_u16(packet, at)? & 0x3fff);
            }
            length => {
                let start = at + 1;
                let label = packet.get(start..start + usize::from(length))?;
                labels.push(String::from_utf8_lossy(label).into_owned());
                at = start + usize::from(length);
            }
        }
    }
    None
}

fn records<T>(packet: &[u8], id: u16, kind: u16, parse: Parse<T>) -> Option<Vec<T>> {
    if read_u16(packet, 0)? != id || packet.get(2)? & 0x80 == 0 {
        return None;
    }
    if packet.get(3)? & 0x0f != 0 {
        return Some(Vec::new());
    }
    let questions = read_u16(packet, 4)?;
    let count = read_u16(packet, 6)?;
    let mut at = 12;
    for _ in 0..questions {
        at = read_name(packet, at)?.1 + 4;
    }
    let mut found = Vec::new();
    for _ in 0..count {
        at = read_name(packet, at)?.1;
        let record = read_u16(packet, at)?;
        let length = usize::from(read_u16(packet, at + 8)?);
        let data = at + 10;
        let end = data + length;
        packet.get(data..end)?;
        if record == kind {
            found.extend(parse(packet, data, end));
        }
        at = end;
    }
    Some(found)
}
