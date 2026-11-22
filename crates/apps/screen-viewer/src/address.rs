const PORT: u16 = 5900;
const DISPLAYS: u16 = 100;
const SCHEME: &str = "vnc://";

/// A computer to connect to: its name, as typed or announced, and port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
}

impl Target {
    /// Reads `vnc://host:port`, `host:port`, `[address]:port` or `host`; a port below 100 is a
    /// display number, as VNC viewers have it.
    pub fn parse(typed: &str) -> Option<Self> {
        let typed = typed.trim();
        let typed = match typed.get(..SCHEME.len()) {
            Some(scheme) if scheme.eq_ignore_ascii_case(SCHEME) => &typed[SCHEME.len()..],
            _ => typed,
        };
        let typed = typed.trim_end_matches('/');
        let (host, port) = match typed.strip_prefix('[') {
            Some(bracketed) => {
                let (host, rest) = bracketed.split_once(']')?;
                (host, rest.strip_prefix(':'))
            }
            None => match typed.matches(':').count() {
                0 => (typed, None),
                1 => {
                    let (host, port) = typed.split_once(':')?;
                    (host, Some(port))
                }
                _ => (typed, None),
            },
        };
        let port = match port {
            None => PORT,
            Some(port) => match port.parse::<u16>().ok()? {
                display if display < DISPLAYS => PORT + display,
                port => port,
            },
        };
        let valid = !host.is_empty()
            && host
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | ':' | '%'));
        valid.then(|| Self {
            host: host.to_owned(),
            port,
        })
    }

    /// How the keyring and the known certificates name this computer.
    pub fn key(&self) -> String {
        match self.host.contains(':') {
            true => format!("[{}]:{}", self.host, self.port),
            false => format!("{}:{}", self.host, self.port),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_name_a_computer_and_its_port() {
        let target = |host: &str, port| {
            Some(Target {
                host: host.to_owned(),
                port,
            })
        };
        assert_eq!(
            Target::parse("vnc://office.local"),
            target("office.local", 5900)
        );
        assert_eq!(
            Target::parse("VNC://office.local:5901/"),
            target("office.local", 5901)
        );
        assert_eq!(Target::parse("192.168.1.4:2"), target("192.168.1.4", 5902));
        assert_eq!(
            Target::parse("[fe80::1%eth0]:5900"),
            target("fe80::1%eth0", 5900)
        );
        assert_eq!(Target::parse("fe80::1"), target("fe80::1", 5900));
        assert_eq!(Target::parse("office;rm -rf"), None);
        assert_eq!(Target::parse(""), None);
    }
}
