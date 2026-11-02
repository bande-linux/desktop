use std::collections::HashSet;
use std::io::Read;
use std::sync::OnceLock;

use url::{Host, Url};

const LIST: &str = "/usr/share/publicsuffix/public_suffix_list.dat";
const LIST_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Site {
    secure: bool,
    domain: String,
}

impl Site {
    pub fn of(url: &Url) -> Option<Self> {
        let domain = match url.host()? {
            Host::Domain(host) => match registrable(host) {
                Some(domain) => domain,
                None => match url.port() {
                    Some(port) => format!("{}:{port}", normal(host)),
                    None => normal(host),
                },
            },
            Host::Ipv4(address) => address.to_string(),
            Host::Ipv6(address) => format!("[{address}]"),
        };
        Some(Self {
            secure: url.scheme() == "https",
            domain,
        })
    }

    pub fn as_str(&self) -> String {
        let scheme = if self.secure { "https" } else { "http" };
        format!("{scheme}://{}", self.domain)
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (scheme, domain) = text.split_once("://")?;
        Some(Self {
            secure: scheme == "https",
            domain: domain.to_owned(),
        })
    }
}

#[derive(Default)]
struct Rules {
    plain: HashSet<String>,
    wildcard: HashSet<String>,
    exception: HashSet<String>,
}

fn ascii(rule: &str) -> Option<String> {
    Url::parse(&format!("http://{rule}/"))
        .ok()?
        .host_str()
        .map(str::to_owned)
}

fn load() -> Option<Rules> {
    let mut text = String::new();
    std::fs::File::open(LIST)
        .ok()?
        .take(LIST_LIMIT)
        .read_to_string(&mut text)
        .ok()?;
    let mut rules = Rules::default();
    for line in text.lines() {
        let Some(rule) = line.split_whitespace().next() else {
            continue;
        };
        if rule.starts_with("//") {
            continue;
        }
        if let Some(rule) = rule.strip_prefix('!') {
            rules.exception.extend(ascii(rule));
        } else if let Some(rule) = rule.strip_prefix("*.") {
            rules.wildcard.extend(ascii(rule));
        } else {
            rules.plain.extend(ascii(rule));
        }
    }
    (!rules.plain.is_empty()).then_some(rules)
}

fn rules() -> Option<&'static Rules> {
    static RULES: OnceLock<Option<Rules>> = OnceLock::new();
    RULES.get_or_init(load).as_ref()
}

fn normal(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn suffix_length(rules: &Rules, labels: &[&str]) -> usize {
    for start in 0..labels.len() {
        let candidate = labels[start..].join(".");
        if rules.exception.contains(&candidate) {
            return labels.len() - start - 1;
        }
        let wildcard = labels
            .get(start + 1..)
            .filter(|rest| !rest.is_empty())
            .is_some_and(|rest| rules.wildcard.contains(&rest.join(".")));
        if rules.plain.contains(&candidate) || wildcard {
            return labels.len() - start;
        }
    }
    1
}

pub(crate) fn registrable(host: &str) -> Option<String> {
    let host = normal(host);
    let rules = rules()?;
    let labels: Vec<&str> = host.split('.').collect();
    let suffix = suffix_length(rules, &labels);
    Some(match labels.len() > suffix {
        true => labels[labels.len() - suffix - 1..].join("."),
        false => host,
    })
}

pub(crate) fn is_public_suffix(domain: &str) -> bool {
    let Some(rules) = rules() else {
        return true;
    };
    let domain = normal(domain);
    let labels: Vec<&str> = domain.split('.').collect();
    suffix_length(rules, &labels) >= labels.len()
}
