//! A nameserver on the loopback interface that answers from a fixed list of records, for tests.

use std::net::{SocketAddr, UdpSocket};

use super::{Resolver, SRV, TXT, read_name};

/// A record the test nameserver answers with.
#[derive(Debug, Clone)]
pub enum Record {
    Srv {
        name: String,
        port: u16,
        target: String,
    },
    Txt {
        name: String,
        text: String,
    },
}

impl Record {
    fn name(&self) -> &str {
        match self {
            Self::Srv { name, .. } | Self::Txt { name, .. } => name,
        }
    }

    fn kind(&self) -> u16 {
        match self {
            Self::Srv { .. } => SRV,
            Self::Txt { .. } => TXT,
        }
    }

    fn data(&self) -> Vec<u8> {
        match self {
            Self::Srv { port, target, .. } => {
                let mut data = vec![0, 0, 0, 0];
                data.extend(port.to_be_bytes());
                data.extend(encode(target));
                data
            }
            Self::Txt { text, .. } => {
                let mut data = vec![text.len().min(255) as u8];
                data.extend(&text.as_bytes()[..text.len().min(255)]);
                data
            }
        }
    }
}

fn encode(name: &str) -> Vec<u8> {
    let mut encoded = Vec::new();
    for label in name
        .trim_end_matches('.')
        .split('.')
        .filter(|label| !label.is_empty())
    {
        encoded.push(label.len() as u8);
        encoded.extend(label.as_bytes());
    }
    encoded.push(0);
    encoded
}

fn answer(query: &[u8], records: &[Record]) -> Option<Vec<u8>> {
    let (name, end) = read_name(query, 12)?;
    let kind = u16::from_be_bytes([*query.get(end)?, *query.get(end + 1)?]);
    let matching = records
        .iter()
        .filter(|record| record.kind() == kind && record.name().eq_ignore_ascii_case(&name))
        .collect::<Vec<_>>();
    let mut packet = query.get(..2)?.to_vec();
    packet.extend([0x81, 0x80, 0, 1]);
    packet.extend((matching.len() as u16).to_be_bytes());
    packet.extend([0, 0, 0, 0]);
    packet.extend(query.get(12..end + 4)?);
    for record in matching {
        let data = record.data();
        packet.extend(encode(record.name()));
        packet.extend(record.kind().to_be_bytes());
        packet.extend([0, 1, 0, 0, 0, 60]);
        packet.extend((data.len() as u16).to_be_bytes());
        packet.extend(data);
    }
    Some(packet)
}

/// Starts a nameserver for `records` on its own thread and returns a resolver that asks it.
pub fn nameserver(records: Vec<Record>) -> std::io::Result<Resolver> {
    let socket = UdpSocket::bind("127.0.0.1:0")?;
    let address: SocketAddr = socket.local_addr()?;
    std::thread::spawn(move || {
        let mut query = [0u8; 512];
        while let Ok((length, from)) = socket.recv_from(&mut query) {
            if let Some(packet) = answer(&query[..length], &records) {
                let _ = socket.send_to(&packet, from);
            }
        }
    });
    Ok(Resolver::new(vec![address]))
}
