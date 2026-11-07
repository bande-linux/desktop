use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketType, bind, ipproto, recvfrom, sendto, socket,
    sockopt,
};

const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const PORT: u16 = 5353;
const IPP: &str = "_ipp._tcp.local";
const IPPS: &str = "_ipps._tcp.local";
const PTR: u16 = 12;
const A: u16 = 1;
const TXT: u16 = 16;
const SRV: u16 = 33;
const CLASS_IN: u16 = 1;
const CLASS_MASK: u16 = 0x7FFF;
const RESPONSE: u16 = 0x8000;
const FIRST_RESEND: Duration = Duration::from_secs(1);
const LONGEST_RESEND: Duration = Duration::from_secs(300);
const MAX_NAME_JUMPS: usize = 32;
const NET_SYS: &str = "/sys/class/net";
const LOOPBACK_FLAG: u32 = 0x8;
const MULTICAST_FLAG: u32 = 0x1000;
const UP_FLAG: u32 = 0x1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub instance: String,
    pub model: String,
    pub location: String,
    pub uuid: String,
    pub plain: Option<String>,
    pub secure: Option<String>,
}

impl Found {
    pub fn id(&self) -> String {
        match self.uuid.is_empty() {
            true => self.instance.clone(),
            false => self.uuid.to_lowercase(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    Pointer {
        service: String,
        instance: String,
        ttl: u32,
    },
    Service {
        instance: String,
        host: String,
        port: u16,
        ttl: u32,
    },
    Text {
        instance: String,
        entries: Vec<(String, String)>,
        ttl: u32,
    },
    Address {
        host: String,
        address: Ipv4Addr,
        ttl: u32,
    },
}

struct Reader<'a> {
    packet: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, count: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(count)?;
        let bytes = self.packet.get(self.at..end)?;
        self.at = end;
        Some(bytes)
    }

    fn short(&mut self) -> Option<u16> {
        let bytes = self.take(2)?;
        Some(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn long(&mut self) -> Option<u32> {
        let bytes = self.take(4)?;
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn name(&mut self) -> Option<String> {
        let (name, end) = read_name(self.packet, self.at)?;
        self.at = end;
        Some(name)
    }
}

fn read_name(packet: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut at = start;
    let mut end = None;
    for _ in 0..MAX_NAME_JUMPS {
        let length = *packet.get(at)?;
        match length {
            0 => {
                return Some((labels.join("."), end.unwrap_or(at + 1)));
            }
            0xC0..=0xFF => {
                let low = *packet.get(at + 1)?;
                end.get_or_insert(at + 2);
                at = usize::from(length & 0x3F) << 8 | usize::from(low);
            }
            1..=63 => {
                let label = packet.get(at + 1..at + 1 + usize::from(length))?;
                labels.push(String::from_utf8_lossy(label).into_owned());
                at += 1 + usize::from(length);
            }
            _ => return None,
        }
    }
    None
}

pub fn parse_packet(packet: &[u8]) -> Option<Vec<Record>> {
    let mut reader = Reader { packet, at: 0 };
    reader.short()?;
    let flags = reader.short()?;
    if flags & RESPONSE == 0 {
        return None;
    }
    let questions = reader.short()?;
    let answers = usize::from(reader.short()?);
    let authority = usize::from(reader.short()?);
    let additional = usize::from(reader.short()?);
    for _ in 0..questions {
        reader.name()?;
        reader.short()?;
        reader.short()?;
    }
    let mut records = Vec::new();
    for _ in 0..answers + authority + additional {
        let name = reader.name()?;
        let kind = reader.short()?;
        let class = reader.short()?;
        let ttl = reader.long()?;
        let length = usize::from(reader.short()?);
        let start = reader.at;
        reader.take(length)?;
        if class & CLASS_MASK != CLASS_IN {
            continue;
        }
        let mut data = Reader { packet, at: start };
        match kind {
            PTR => {
                if let Some((instance, _)) = read_name(packet, start) {
                    records.push(Record::Pointer {
                        service: name.to_lowercase(),
                        instance,
                        ttl,
                    });
                }
            }
            SRV => {
                data.short()?;
                data.short()?;
                let port = data.short()?;
                let host = data.name()?;
                records.push(Record::Service {
                    instance: name,
                    host,
                    port,
                    ttl,
                });
            }
            TXT => {
                let mut entries = Vec::new();
                let end = start + length;
                while data.at < end {
                    let size = usize::from(*data.take(1)?.first()?);
                    let entry = String::from_utf8_lossy(data.take(size)?).into_owned();
                    if let Some((key, value)) = entry.split_once('=') {
                        entries.push((key.to_lowercase(), value.to_owned()));
                    }
                }
                records.push(Record::Text {
                    instance: name,
                    entries,
                    ttl,
                });
            }
            A if length == 4 => {
                let octets = data.take(4)?;
                records.push(Record::Address {
                    host: name,
                    address: Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]),
                    ttl,
                });
            }
            _ => {}
        }
    }
    Some(records)
}

fn write_name(packet: &mut Vec<u8>, name: &str) {
    for label in name.split('.').filter(|label| !label.is_empty()) {
        packet.push(u8::try_from(label.len()).unwrap_or(63));
        packet.extend(label.bytes().take(63));
    }
    packet.push(0);
}

fn query(names: &[(&str, u16)]) -> Vec<u8> {
    let mut packet = vec![0, 0, 0, 0];
    packet.extend(u16::try_from(names.len()).unwrap_or(0).to_be_bytes());
    packet.extend([0; 6]);
    for (name, kind) in names {
        write_name(&mut packet, name);
        packet.extend(kind.to_be_bytes());
        packet.extend(CLASS_IN.to_be_bytes());
    }
    packet
}

fn up_interfaces() -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(NET_SYS) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let read = |file: &str| std::fs::read_to_string(entry.path().join(file)).ok();
            let flags = read("flags")?;
            let flags = u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).ok()?;
            let usable =
                flags & UP_FLAG != 0 && flags & MULTICAST_FLAG != 0 && flags & LOOPBACK_FLAG == 0;
            if !usable {
                return None;
            }
            read("ifindex")?.trim().parse().ok()
        })
        .collect()
}

#[derive(Default)]
struct Cache {
    pointers: HashMap<(String, String), Instant>,
    services: HashMap<String, (String, u16, Instant)>,
    texts: HashMap<String, Vec<(String, String)>>,
    addresses: HashMap<String, (Ipv4Addr, Instant)>,
}

impl Cache {
    fn absorb(&mut self, records: Vec<Record>, now: Instant) {
        for record in records {
            match record {
                Record::Pointer {
                    service,
                    instance,
                    ttl,
                } if service == IPP || service == IPPS => {
                    let key = (service, instance);
                    match ttl {
                        0 => {
                            self.pointers.remove(&key);
                        }
                        ttl => {
                            self.pointers
                                .insert(key, now + Duration::from_secs(u64::from(ttl)));
                        }
                    }
                }
                Record::Service {
                    instance,
                    host,
                    port,
                    ttl,
                } => {
                    self.services.insert(
                        instance,
                        (host, port, now + Duration::from_secs(u64::from(ttl))),
                    );
                }
                Record::Text {
                    instance, entries, ..
                } => {
                    self.texts.insert(instance, entries);
                }
                Record::Address { host, address, ttl } => {
                    self.addresses
                        .insert(host, (address, now + Duration::from_secs(u64::from(ttl))));
                }
                Record::Pointer { .. } => {}
            }
        }
    }

    fn expire(&mut self, now: Instant) {
        self.pointers.retain(|_, until| *until > now);
    }

    fn next_expiry(&self) -> Option<Instant> {
        self.pointers.values().copied().min()
    }

    fn missing(&self) -> Vec<(String, u16)> {
        let mut missing = Vec::new();
        for (_, instance_name) in self.pointers.keys() {
            let full = instance_name;
            match self.services.get(full) {
                None => missing.push((full.clone(), SRV)),
                Some((host, _, _)) if !self.addresses.contains_key(host) => {
                    missing.push((host.clone(), A));
                }
                Some(_) => {}
            }
            if !self.texts.contains_key(full) {
                missing.push((full.clone(), TXT));
            }
        }
        missing.sort();
        missing.dedup();
        missing
    }

    fn found(&self) -> Vec<Found> {
        let mut printers: HashMap<String, Found> = HashMap::new();
        for (service, instance) in self.pointers.keys() {
            let Some((host, port, _)) = self.services.get(instance) else {
                continue;
            };
            let Some((address, _)) = self.addresses.get(host) else {
                continue;
            };
            let empty = Vec::new();
            let text = self.texts.get(instance).unwrap_or(&empty);
            let get = |key: &str| {
                text.iter()
                    .find(|(name, _)| name == key)
                    .map_or("", |(_, value)| value.as_str())
            };
            let resource = match get("rp") {
                "" => "ipp/print",
                other => other,
            }
            .trim_start_matches('/');
            let short = instance
                .strip_suffix(&format!(".{service}"))
                .unwrap_or(instance)
                .to_owned();
            let entry = printers.entry(short.clone()).or_insert_with(|| Found {
                instance: short,
                model: get("ty").to_owned(),
                location: get("note").to_owned(),
                uuid: get("uuid").to_owned(),
                plain: None,
                secure: None,
            });
            match service.as_str() {
                IPPS => entry.secure = Some(format!("ipps://{address}:{port}/{resource}")),
                _ => entry.plain = Some(format!("ipp://{address}:{port}/{resource}")),
            }
        }
        let mut list: Vec<Found> = printers.into_values().collect();
        list.sort_by_key(|printer| printer.instance.to_lowercase());
        list
    }
}

fn open_socket(interfaces: &[u32]) -> rustix::io::Result<OwnedFd> {
    let fd = socket(AddressFamily::INET, SocketType::DGRAM, Some(ipproto::UDP))?;
    sockopt::set_socket_reuseaddr(&fd, true)?;
    sockopt::set_socket_reuseport(&fd, true)?;
    bind(&fd, &SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT))?;
    sockopt::set_ip_multicast_ttl(&fd, 255)?;
    for interface in interfaces {
        sockopt::set_ip_add_membership_with_ifindex(
            &fd,
            &GROUP,
            &Ipv4Addr::UNSPECIFIED,
            *interface,
        )?;
    }
    Ok(fd)
}

fn send_query(fd: impl AsFd, interfaces: &[u32], cache: &Cache) {
    let missing = cache.missing();
    let mut names: Vec<(&str, u16)> = vec![(IPP, PTR), (IPPS, PTR)];
    names.extend(missing.iter().map(|(name, kind)| (name.as_str(), *kind)));
    let packet = query(&names);
    let group = SocketAddrV4::new(GROUP, PORT);
    for interface in interfaces {
        if let Err(error) = sockopt::set_ip_multicast_if_with_ifindex(
            &fd,
            &GROUP,
            &Ipv4Addr::UNSPECIFIED,
            *interface,
        ) {
            tracing::debug!("cannot use interface {interface} for printer discovery: {error}");
            continue;
        }
        if let Err(error) = sendto(&fd, &packet, SendFlags::empty(), &group) {
            tracing::debug!("cannot ask interface {interface} for printers: {error}");
        }
    }
}

pub fn browse(
    wake: impl AsFd,
    mut changed: impl FnMut(Vec<Found>) -> bool,
) -> rustix::io::Result<()> {
    let mut interfaces = up_interfaces();
    let mut fd = open_socket(&interfaces)?;
    let mut cache = Cache::default();
    let mut resend = FIRST_RESEND;
    let mut next_send = Instant::now();
    let mut seen = Vec::new();
    let mut buffer = vec![0u8; 9000];
    loop {
        let now = Instant::now();
        cache.expire(now);
        if now >= next_send {
            let current = up_interfaces();
            if current != interfaces {
                interfaces = current;
                fd = open_socket(&interfaces)?;
                cache.services.clear();
                resend = FIRST_RESEND;
            }
            send_query(&fd, &interfaces, &cache);
            next_send = now + resend;
            resend = (resend * 2).min(LONGEST_RESEND);
        }
        let found = cache.found();
        if found != seen {
            seen.clone_from(&found);
            if !changed(found) {
                return Ok(());
            }
        }
        let deadline = cache
            .next_expiry()
            .map_or(next_send, |expiry| expiry.min(next_send));
        let wait = deadline.saturating_duration_since(Instant::now());
        let timeout = Timespec {
            tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(wait.subsec_nanos()),
        };
        let mut fds = [
            PollFd::new(&fd, PollFlags::IN),
            PollFd::new(&wake, PollFlags::IN),
        ];
        match poll(&mut fds, Some(&timeout)) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error),
        }
        let (readable, woken) = (
            fds[0].revents().contains(PollFlags::IN),
            !fds[1].revents().is_empty(),
        );
        if woken {
            return Ok(());
        }
        if !readable {
            continue;
        }
        let (length, _, _) = recvfrom(&fd, &mut buffer, RecvFlags::empty())?;
        if let Some(records) = buffer.get(..length).and_then(parse_packet) {
            cache.absorb(records, Instant::now());
            if !cache.missing().is_empty() && resend > FIRST_RESEND * 4 {
                next_send = next_send.min(Instant::now() + FIRST_RESEND);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_printer_announcement_with_compressed_names_resolves_to_its_address() {
        let mut packet = vec![0, 0, 0x84, 0, 0, 0, 0, 4, 0, 0, 0, 0];
        let service_at = packet.len();
        write_name(&mut packet, IPP);
        packet.extend(PTR.to_be_bytes());
        packet.extend(CLASS_IN.to_be_bytes());
        packet.extend(4500u32.to_be_bytes());
        let label = b"Office";
        packet.extend(u16::try_from(label.len() + 3).unwrap().to_be_bytes());
        let instance_at = packet.len();
        packet.push(u8::try_from(label.len()).unwrap());
        packet.extend(label);
        packet.extend([0xC0, u8::try_from(service_at).unwrap()]);

        packet.extend([0xC0, u8::try_from(instance_at).unwrap()]);
        packet.extend(SRV.to_be_bytes());
        packet.extend((CLASS_IN | 0x8000).to_be_bytes());
        packet.extend(120u32.to_be_bytes());
        let mut host = Vec::new();
        write_name(&mut host, "office.local");
        packet.extend(u16::try_from(6 + host.len()).unwrap().to_be_bytes());
        packet.extend([0, 0, 0, 0, 0x02, 0x77]);
        let host_at = packet.len();
        packet.extend(&host);

        packet.extend([0xC0, u8::try_from(instance_at).unwrap()]);
        packet.extend(TXT.to_be_bytes());
        packet.extend(CLASS_IN.to_be_bytes());
        packet.extend(4500u32.to_be_bytes());
        let text = b"\x0erp=ipp/printer\x07ty=Acme";
        packet.extend(u16::try_from(text.len()).unwrap().to_be_bytes());
        packet.extend(text);

        packet.extend([0xC0, u8::try_from(host_at).unwrap()]);
        packet.extend(A.to_be_bytes());
        packet.extend(CLASS_IN.to_be_bytes());
        packet.extend(120u32.to_be_bytes());
        packet.extend(4u16.to_be_bytes());
        packet.extend([192, 168, 1, 40]);

        let mut cache = Cache::default();
        cache.absorb(parse_packet(&packet).unwrap(), Instant::now());
        let found = cache.found();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].instance, "Office");
        assert_eq!(found[0].model, "Acme");
        assert_eq!(
            found[0].plain.as_deref(),
            Some("ipp://192.168.1.40:631/ipp/printer")
        );
        assert!(cache.missing().is_empty());
    }
}
