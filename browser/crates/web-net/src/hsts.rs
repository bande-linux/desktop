use std::collections::HashMap;

use url::{Host, Url};

const MAX_HOSTS: usize = 10_000;

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
            self.hosts
                .get(name)
                .is_some_and(|policy| policy.expires > now && (depth == 0 || policy.subdomains))
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
