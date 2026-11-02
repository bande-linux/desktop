use url::{Host, Url};

use crate::date::parse_cookie_date;
use crate::site::{Site, is_public_suffix};

const MAX_COOKIE: usize = 4096;
const MAX_PER_DOMAIN: usize = 180;
const MAX_COOKIES: usize = 3000;
const MAX_AGE: u64 = 400 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SameSite {
    Strict,
    Lax,
    None,
}

#[derive(Debug, Clone, PartialEq)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
    host_only: bool,
    path: String,
    expires: Option<u64>,
    secure: bool,
    http_only: bool,
    same_site: SameSite,
    partition: Option<Site>,
    created: u64,
}

impl Cookie {
    fn same_identity(&self, other: &Cookie) -> bool {
        self.name == other.name
            && self.domain == other.domain
            && self.path == other.path
            && self.partition == other.partition
    }

    fn expired(&self, now: u64) -> bool {
        self.expires.is_some_and(|expires| expires <= now)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Context<'a> {
    pub(crate) top_level: &'a Site,
    pub(crate) first_party: bool,
    pub(crate) same_site: bool,
    pub(crate) navigation: bool,
}

#[derive(Default)]
pub(crate) struct Jar {
    cookies: Vec<Cookie>,
    next: u64,
    changed: bool,
}

fn host_of(url: &Url) -> Option<(String, bool)> {
    match url.host()? {
        Host::Domain(domain) => Some((domain.trim_end_matches('.').to_ascii_lowercase(), false)),
        Host::Ipv4(address) => Some((address.to_string(), true)),
        Host::Ipv6(address) => Some((format!("[{address}]"), true)),
    }
}

fn domain_matches(host: &str, domain: &str, host_is_ip: bool) -> bool {
    host == domain
        || (!host_is_ip
            && host.len() > domain.len()
            && host.ends_with(domain)
            && host.as_bytes().get(host.len() - domain.len() - 1) == Some(&b'.'))
}

fn default_path(url: &Url) -> String {
    let path = url.path();
    match path.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(end) => path[..end].to_owned(),
    }
}

fn path_matches(request: &str, cookie: &str) -> bool {
    request == cookie
        || (request.starts_with(cookie)
            && (cookie.ends_with('/') || request.as_bytes().get(cookie.len()) == Some(&b'/')))
}

fn trustworthy(url: &Url) -> bool {
    url.scheme() == "https"
        || matches!(url.host(), Some(Host::Domain("localhost")))
        || matches!(url.host(), Some(Host::Ipv4(address)) if address.is_loopback())
}

fn clean(text: &str) -> bool {
    !text
        .chars()
        .any(|character| character.is_control() && character != '\t')
}

impl Jar {
    pub(crate) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    pub(crate) fn store(&mut self, url: &Url, header: &str, context: Context<'_>, now: u64) {
        let Some(cookie) = self.parse(url, header, context, now) else {
            return;
        };
        if !trustworthy(url)
            && self.cookies.iter().any(|known| {
                known.secure
                    && known.name == cookie.name
                    && domain_matches(&known.domain, &cookie.domain, false)
            })
        {
            return;
        }
        let created = match self
            .cookies
            .iter()
            .position(|known| known.same_identity(&cookie))
        {
            Some(index) => {
                let old = self.cookies.remove(index);
                if old.http_only && !cookie.http_only {
                    self.cookies.insert(index, old);
                    return;
                }
                old.created
            }
            None => {
                self.next += 1;
                self.next
            }
        };
        self.changed = true;
        if cookie.expired(now) {
            return;
        }
        self.cookies.push(Cookie { created, ..cookie });
        self.evict(now);
    }

    fn parse(&self, url: &Url, header: &str, context: Context<'_>, now: u64) -> Option<Cookie> {
        if header.len() > MAX_COOKIE || !clean(header) {
            return None;
        }
        let (pair, attributes) = header.split_once(';').unwrap_or((header, ""));
        let (name, value) = match pair.split_once('=') {
            Some((name, value)) => (name.trim(), value.trim()),
            None => ("", pair.trim()),
        };
        if name.is_empty() && value.is_empty() {
            return None;
        }
        let (host, host_is_ip) = host_of(url)?;
        let mut expires = None;
        let mut max_age = None;
        let mut domain = None;
        let mut path = None;
        let mut secure = false;
        let mut http_only = false;
        let mut same_site = SameSite::Lax;
        let mut partitioned = false;
        for attribute in attributes.split(';') {
            let (key, value) = attribute.split_once('=').unwrap_or((attribute, ""));
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "expires" => expires = parse_cookie_date(value).or(expires),
                "max-age" => {
                    if let Ok(seconds) = value.parse::<i64>() {
                        max_age = Some(seconds);
                    }
                }
                "domain" if !value.is_empty() => {
                    domain = Some(value.trim_start_matches('.').to_ascii_lowercase());
                }
                "path" => path = value.starts_with('/').then(|| value.to_owned()),
                "secure" => secure = true,
                "httponly" => http_only = true,
                "samesite" => {
                    same_site = match value.to_ascii_lowercase().as_str() {
                        "strict" => SameSite::Strict,
                        "none" => SameSite::None,
                        _ => SameSite::Lax,
                    }
                }
                "partitioned" => partitioned = true,
                _ => {}
            }
        }
        let expires = match max_age {
            Some(seconds) if seconds <= 0 => Some(0),
            Some(seconds) => Some(now.saturating_add((seconds as u64).min(MAX_AGE))),
            None => expires.map(|expires| expires.min(now.saturating_add(MAX_AGE))),
        };
        let (domain, host_only) = match domain {
            Some(domain) if domain == host => (domain, true),
            Some(domain) if host_is_ip || is_public_suffix(&domain) => return None,
            Some(domain) if domain_matches(&host, &domain, host_is_ip) => (domain, false),
            Some(_) => return None,
            None => (host, true),
        };
        if secure && !trustworthy(url) {
            return None;
        }
        if name.starts_with("__Secure-") && !secure {
            return None;
        }
        if name.starts_with("__Host-") && (!secure || !host_only || path.as_deref() != Some("/")) {
            return None;
        }
        if same_site == SameSite::None && !secure {
            return None;
        }
        if partitioned && !secure {
            return None;
        }
        let partition = match (partitioned, context.first_party) {
            (true, _) => Some(context.top_level.clone()),
            (false, true) => None,
            (false, false) => return None,
        };
        Some(Cookie {
            name: name.to_owned(),
            value: value.to_owned(),
            domain,
            host_only,
            path: path.unwrap_or_else(|| default_path(url)),
            expires,
            secure,
            http_only,
            same_site,
            partition,
            created: 0,
        })
    }

    fn evict(&mut self, now: u64) {
        self.cookies.retain(|cookie| !cookie.expired(now));
        let Some(last) = self.cookies.last().map(|cookie| cookie.domain.clone()) else {
            return;
        };
        while self
            .cookies
            .iter()
            .filter(|cookie| cookie.domain == last)
            .count()
            > MAX_PER_DOMAIN
        {
            if let Some(oldest) = self.cookies.iter().position(|cookie| cookie.domain == last) {
                self.cookies.remove(oldest);
            }
        }
        if self.cookies.len() > MAX_COOKIES {
            let excess = self.cookies.len() - MAX_COOKIES;
            self.cookies.drain(..excess);
        }
    }

    pub(crate) fn header(&mut self, url: &Url, context: Context<'_>, now: u64) -> Option<String> {
        let (host, host_is_ip) = host_of(url)?;
        let secure = trustworthy(url);
        if self.cookies.iter().any(|cookie| cookie.expired(now)) {
            self.cookies.retain(|cookie| !cookie.expired(now));
            self.changed = true;
        }
        let mut sent: Vec<&Cookie> = self
            .cookies
            .iter()
            .filter(|cookie| match cookie.host_only {
                true => cookie.domain == host,
                false => domain_matches(&host, &cookie.domain, host_is_ip),
            })
            .filter(|cookie| path_matches(url.path(), &cookie.path))
            .filter(|cookie| secure || !cookie.secure)
            .filter(|cookie| match &cookie.partition {
                Some(partition) => partition == context.top_level,
                None => context.first_party,
            })
            .filter(|cookie| match cookie.same_site {
                SameSite::Strict => context.same_site,
                SameSite::Lax => context.same_site || context.navigation,
                SameSite::None => true,
            })
            .collect();
        if sent.is_empty() {
            return None;
        }
        sent.sort_by(|a, b| {
            b.path
                .len()
                .cmp(&a.path.len())
                .then(a.created.cmp(&b.created))
        });
        Some(
            sent.iter()
                .map(|cookie| match cookie.name.is_empty() {
                    true => cookie.value.clone(),
                    false => format!("{}={}", cookie.name, cookie.value),
                })
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    pub(crate) fn save(&self) -> String {
        let mut text = String::new();
        for cookie in &self.cookies {
            let Some(expires) = cookie.expires else {
                continue;
            };
            let same_site = match cookie.same_site {
                SameSite::Strict => "strict",
                SameSite::Lax => "lax",
                SameSite::None => "none",
            };
            let partition = cookie
                .partition
                .as_ref()
                .map_or("-".to_owned(), Site::as_str);
            let fields = [
                cookie.name.clone(),
                cookie.value.clone(),
                cookie.domain.clone(),
                u8::from(cookie.host_only).to_string(),
                cookie.path.clone(),
                expires.to_string(),
                u8::from(cookie.secure).to_string(),
                u8::from(cookie.http_only).to_string(),
                same_site.to_owned(),
                partition,
                cookie.created.to_string(),
            ];
            text.push_str(&fields.join("\t"));
            text.push('\n');
        }
        text
    }

    pub(crate) fn load(text: &str, now: u64) -> Self {
        let mut jar = Self::default();
        for line in text.lines().take(MAX_COOKIES) {
            let fields: Vec<&str> = line.split('\t').collect();
            let [
                name,
                value,
                domain,
                host_only,
                path,
                expires,
                secure,
                http_only,
                same_site,
                partition,
                created,
            ] = fields.as_slice()
            else {
                continue;
            };
            let (Ok(expires), Ok(created)) = (expires.parse::<u64>(), created.parse::<u64>())
            else {
                continue;
            };
            let same_site = match *same_site {
                "strict" => SameSite::Strict,
                "none" => SameSite::None,
                _ => SameSite::Lax,
            };
            let cookie = Cookie {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
                domain: (*domain).to_owned(),
                host_only: *host_only == "1",
                path: (*path).to_owned(),
                expires: Some(expires),
                secure: *secure == "1",
                http_only: *http_only == "1",
                same_site,
                partition: (*partition != "-")
                    .then(|| Site::parse(partition))
                    .flatten(),
                created,
            };
            if !cookie.expired(now) {
                jar.next = jar.next.max(created);
                jar.cookies.push(cookie);
            }
        }
        jar
    }
}
