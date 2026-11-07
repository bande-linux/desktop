#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Smb,
    Sftp,
    Dav,
    Davs,
    Nfs,
}

impl Protocol {
    fn scheme(self) -> &'static str {
        match self {
            Self::Smb => "smb",
            Self::Sftp => "sftp",
            Self::Dav => "dav",
            Self::Davs => "davs",
            Self::Nfs => "nfs",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub protocol: Protocol,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub path: Vec<String>,
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte != b'%' {
            out.push(byte);
            index += 1;
            continue;
        }
        let digits = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
        out.push(u8::from_str_radix(digits, 16).ok()?);
        index += 3;
    }
    String::from_utf8(out).ok()
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(char::from(byte));
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn valid_host(host: &str) -> bool {
    match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(address) => address.parse::<std::net::Ipv6Addr>().is_ok(),
        None => {
            !host.is_empty()
                && !host.starts_with('-')
                && host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        }
    }
}

fn host_and_port(text: &str) -> Option<(String, Option<u16>)> {
    let (host, port) = match text.rfind(':').filter(|at| !text[*at..].contains(']')) {
        Some(at) => (&text[..at], Some(text[at + 1..].parse().ok()?)),
        None => (text, None),
    };
    valid_host(host).then(|| (host.to_owned(), port))
}

impl Server {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (scheme, rest) = text.split_once("://").unwrap_or(("smb", text));
        let protocol = match scheme.to_ascii_lowercase().as_str() {
            "smb" | "cifs" => Protocol::Smb,
            "sftp" | "ssh" => Protocol::Sftp,
            "dav" | "http" | "webdav" => Protocol::Dav,
            "davs" | "https" | "webdavs" => Protocol::Davs,
            "nfs" => Protocol::Nfs,
            _ => return None,
        };
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let (user, address) = match authority.rsplit_once('@') {
            Some((info, address)) => {
                let name = info.split_once(':').map_or(info, |(name, _)| name);
                (Some(percent_decode(name)?), address)
            }
            None => (None, authority),
        };
        let (host, port) = host_and_port(address)?;
        let path = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(percent_decode)
            .collect::<Option<Vec<_>>>()?;
        let unsafe_segment = |segment: &String| {
            segment == ".." || segment == "." || segment.chars().any(char::is_control)
        };
        if path.iter().any(unsafe_segment) {
            return None;
        }
        Some(Self {
            protocol,
            user: user.filter(|user| !user.is_empty()),
            host,
            port,
            path,
        })
    }

    pub fn origin(&self) -> String {
        let scheme = self.protocol.scheme();
        match self.port {
            Some(port) => format!("{scheme}://{}:{port}", self.host),
            None => format!("{scheme}://{}", self.host),
        }
    }

    pub fn url(&self) -> String {
        let mut url = self.origin();
        for segment in &self.path {
            url.push('/');
            url.push_str(&percent_encode(segment));
        }
        url
    }

    pub fn volume_name(&self) -> String {
        let name = match self.protocol {
            Protocol::Smb => self.path.first(),
            Protocol::Sftp | Protocol::Dav | Protocol::Davs | Protocol::Nfs => self.path.last(),
        };
        name.cloned()
            .unwrap_or_else(|| self.host.clone())
            .replace([',', '/'], "-")
            .trim_start_matches('-')
            .to_owned()
    }

    pub fn unc(&self) -> Option<String> {
        let share = self.path.first()?;
        Some(format!("//{}/{share}", self.host))
    }

    pub fn inner_path(&self) -> &[String] {
        match self.protocol {
            Protocol::Smb => self.path.get(1..).unwrap_or_default(),
            Protocol::Sftp | Protocol::Dav | Protocol::Davs | Protocol::Nfs => &[],
        }
    }

    pub fn dav_url(&self) -> String {
        let scheme = match self.protocol {
            Protocol::Davs => "https",
            Protocol::Smb | Protocol::Sftp | Protocol::Dav | Protocol::Nfs => "http",
        };
        let address = match self.port {
            Some(port) => format!("{}:{port}", self.host),
            None => self.host.clone(),
        };
        let path: String = self
            .path
            .iter()
            .map(|segment| format!("/{}", percent_encode(segment)))
            .collect();
        format!("{scheme}://{address}{path}/")
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or(match self.protocol {
            Protocol::Smb => 445,
            Protocol::Sftp => 22,
            Protocol::Dav => 80,
            Protocol::Davs => 443,
            Protocol::Nfs => 2049,
        })
    }

    pub fn nfs_source(&self) -> String {
        format!("{}:/{}", self.host, self.path.join("/"))
    }

    pub fn sftp_source(&self) -> String {
        let path = match self.path.is_empty() {
            true => String::new(),
            false => format!("/{}", self.path.join("/")),
        };
        match &self.user {
            Some(user) => format!("{user}@{}:{path}", self.host),
            None => format!("{}:{path}", self.host),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_become_servers() {
        let plain = Server::parse("nas.local").unwrap();
        assert_eq!(plain.protocol, Protocol::Smb);
        assert_eq!(plain.origin(), "smb://nas.local");
        let share = Server::parse("smb://ann@nas:4455/My%20Media/films/").unwrap();
        assert_eq!(share.user.as_deref(), Some("ann"));
        assert_eq!(share.unc().as_deref(), Some("//nas/My Media"));
        assert_eq!(share.inner_path(), ["films"]);
        assert_eq!(share.url(), "smb://nas:4455/My%20Media/films");
        let ipv6 = Server::parse("sftp://[fe80::1]:2222/srv").unwrap();
        assert_eq!(ipv6.sftp_source(), "[fe80::1]:/srv");
        assert_eq!(ipv6.port, Some(2222));
        let dav = Server::parse("https://cloud.example/remote.php/dav").unwrap();
        assert_eq!(dav.dav_url(), "https://cloud.example/remote.php/dav/");
        assert_eq!(dav.volume_name(), "dav");
        let nfs = Server::parse("nfs://nas/srv/media").unwrap();
        assert_eq!(nfs.nfs_source(), "nas:/srv/media");
        assert_eq!(nfs.volume_name(), "media");
    }

    #[test]
    fn addresses_that_could_inject_are_refused() {
        for bad in [
            "sftp://-oProxyCommand=x/",
            "smb://a/../etc",
            "smb://a b/c",
            "gopher://x",
        ] {
            assert_eq!(Server::parse(bad), None, "{bad}");
        }
    }
}
