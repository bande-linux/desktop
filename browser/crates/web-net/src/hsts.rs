use std::collections::HashMap;

use url::{Host, Url};

const MAX_HOSTS: usize = 10_000;
static PRELOADED: &[u8] = include_bytes!("../data/hsts-preload.txt");

struct Policy {
    expires: u64,
    subdomains: bool,
}

#[derive(Default)]
pub(crate) struct Hsts {
    hosts: HashMap<String, Policy>,
    changed: bool,
}

fn domain(url: &Url) -> Option<String> {
    match url.host()? {
        Host::Domain(domain) => Some(domain.trim_end_matches('.').to_ascii_lowercase()),
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

impl Hsts {
    pub(crate) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub(crate) fn upgrade(&self, url: &mut Url, now: u64) {
        if url.scheme() != "http" {
            return;
        }
        let Some(host) = domain(url) else {
            return;
        };
        let known = std::iter::successors(Some(host.as_str()), |name| {
            name.split_once('.').map(|(_, parent)| parent)
        })
        .enumerate()
        .any(|(depth, name)| {
            let dynamic = self
                .hosts
                .get(name)
                .and_then(|policy| (policy.expires > now).then_some(policy.subdomains));
            dynamic
                .or_else(|| preloaded(PRELOADED, name))
                .is_some_and(|subdomains| depth == 0 || subdomains)
        });
        if known && url.set_scheme("https").is_ok() && url.port() == Some(80) {
            let _ = url.set_port(None);
        }
    }

    pub(crate) fn observe(&mut self, url: &Url, header: &str, now: u64) {
        if url.scheme() != "https" {
            return;
        }
        let Some(host) = domain(url) else {
            return;
        };
        let mut max_age = None;
        let mut subdomains = false;
        for directive in header.split(';') {
            let (name, value) = directive.split_once('=').unwrap_or((directive, ""));
            match name.trim().to_ascii_lowercase().as_str() {
                "max-age" => max_age = value.trim().trim_matches('"').parse::<u64>().ok(),
                "includesubdomains" => subdomains = true,
                _ => {}
            }
        }
        let Some(max_age) = max_age else {
            return;
        };
        self.changed = true;
        if max_age == 0 {
            self.hosts.remove(&host);
            return;
        }
        if self.hosts.len() >= MAX_HOSTS && !self.hosts.contains_key(&host) {
            self.hosts.retain(|_, policy| policy.expires > now);
            if self.hosts.len() >= MAX_HOSTS {
                return;
            }
        }
        self.hosts.insert(
            host,
            Policy {
                expires: now.saturating_add(max_age),
                subdomains,
            },
        );
    }

    pub(crate) fn save(&self) -> String {
        self.hosts
            .iter()
            .map(|(host, policy)| {
                format!(
                    "{host}\t{}\t{}\n",
                    policy.expires,
                    u8::from(policy.subdomains)
                )
            })
            .collect()
    }

    pub(crate) fn load(text: &str, now: u64) -> Self {
        let hosts = text
            .lines()
            .take(MAX_HOSTS)
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let host = fields.next()?.to_owned();
                let expires = fields.next()?.parse::<u64>().ok()?;
                let subdomains = fields.next()? == "1";
                (expires > now).then_some((
                    host,
                    Policy {
                        expires,
                        subdomains,
                    },
                ))
            })
            .collect();
        Self {
            hosts,
            changed: false,
        }
    }
}

pub(crate) fn preloaded(table: &[u8], name: &str) -> Option<bool> {
    let name = name.as_bytes();
    let (mut low, mut high) = (0, table.len());
    while low < high {
        let middle = low + (high - low) / 2;
        let start = table
            .get(..middle)?
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(0, |newline| newline + 1)
            .max(low);
        let end = table
            .get(start..)?
            .iter()
            .position(|&byte| byte == b'\n')
            .map_or(table.len(), |newline| start + newline);
        let line = table.get(start..end)?;
        let (key, flag) = match line.iter().position(|&byte| byte == b'\t') {
            Some(tab) => (line.get(..tab)?, line.get(tab + 1..)?),
            None => (line, &b""[..]),
        };
        match key.cmp(name) {
            std::cmp::Ordering::Equal => return Some(flag == b"s"),
            std::cmp::Ordering::Less => low = end + 1,
            std::cmp::Ordering::Greater => high = start,
        }
    }
    None
}
