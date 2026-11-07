use std::collections::BTreeSet;
use std::fs;

const REMOVED: &str = "<removed>";
const PATH: &str = "<path>";
const URL: &str = "<url>";
const ADDRESS: &str = "<address>";
const QUOTED: &str = "<text>";
const EMAIL: &str = "<email>";
const HOST: &str = "<host>";
const FIRST_USER: u32 = 1000;
const NOBODY: u32 = 65534;
const SHORTEST_SECRET: usize = 3;
const LONGEST_QUOTED: usize = 32;
const OWN_PATHS: [&str; 15] = [
    "/usr/",
    "/lib/",
    "/lib64/",
    "/bin/",
    "/sbin/",
    "/etc/bande/",
    "/etc/systemd/",
    "/sys/",
    "/proc/",
    "/dev/",
    "/run/bande/",
    "/run/systemd/",
    "/var/lib/bande/",
    "/var/lib/systemd/",
    "/boot/",
];
const NAME_ENDINGS: [&str; 22] = [
    "c", "cc", "conf", "desktop", "h", "json", "ko", "mount", "path", "rs", "scope", "service",
    "slice", "so", "socket", "swap", "target", "timer", "toml", "wl", "wlr", "xml",
];
const HOST_ENDINGS: [&str; 14] = [
    "com", "de", "fi", "home", "internal", "io", "lan", "local", "net", "no", "org", "se", "uk",
    "dk",
];
const SERIAL_FILES: [&str; 6] = [
    "/proc/device-tree/serial-number",
    "/sys/class/dmi/id/product_serial",
    "/sys/class/dmi/id/board_serial",
    "/sys/class/dmi/id/chassis_serial",
    "/sys/class/dmi/id/product_uuid",
    "/etc/machine-id",
];
const HOSTNAME_FILE: &str = "/proc/sys/kernel/hostname";
const PASSWD: &str = "/etc/passwd";
const IWD_NETWORKS: &str = "/var/lib/iwd";
const NETWORK_MANAGER: &str = "/etc/NetworkManager/system-connections";

#[derive(Default)]
pub struct Scrubber {
    secrets: Vec<String>,
}

fn decode_iwd_name(stem: &str) -> Option<String> {
    let hex = stem.strip_prefix('=')?;
    let bytes: Option<Vec<u8>> = (0..hex.len() / 2)
        .map(|at| u8::from_str_radix(hex.get(at * 2..at * 2 + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

fn network_names() -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(IWD_NETWORKS) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                names.push(decode_iwd_name(stem).unwrap_or_else(|| stem.to_owned()));
            }
        }
    }
    if let Ok(entries) = fs::read_dir(NETWORK_MANAGER) {
        for entry in entries.flatten() {
            let text = fs::read_to_string(entry.path()).unwrap_or_default();
            names.extend(
                text.lines()
                    .filter_map(|line| line.strip_prefix("ssid="))
                    .map(str::to_owned),
            );
        }
    }
    names
}

fn accounts() -> Vec<String> {
    let passwd = fs::read_to_string(PASSWD).unwrap_or_default();
    let mut found = Vec::new();
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let uid: Option<u32> = fields.get(2).and_then(|uid| uid.parse().ok());
        if uid.is_some_and(|uid| (FIRST_USER..NOBODY).contains(&uid)) {
            found.extend(fields.first().map(|name| (*name).to_owned()));
            found.extend(fields.get(4).map(|real| (*real).to_owned()));
        }
    }
    found
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn replace_words(text: &str, secret: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut before = None;
    while let Some(at) = rest.find(secret) {
        let end = at + secret.len();
        let left = rest[..at].bytes().next_back().or(before);
        let right = rest[end..].bytes().next();
        out.push_str(&rest[..at]);
        if left.is_some_and(is_word) || right.is_some_and(is_word) {
            out.push_str(secret);
        } else {
            out.push_str(REMOVED);
        }
        before = Some(secret.as_bytes()[secret.len() - 1]);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn is_mac(token: &str) -> bool {
    let groups: Vec<&str> = token.split([':', '-']).collect();
    groups.len() == 6
        && groups
            .iter()
            .all(|group| group.len() == 2 && group.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn is_ipv4(token: &str) -> bool {
    let address = token.split(['/', ':']).next().unwrap_or(token);
    address.parse::<std::net::Ipv4Addr>().is_ok()
}

fn is_ipv6(token: &str) -> bool {
    let address = token.trim_matches(['[', ']']);
    let address = address.split(['/', '%']).next().unwrap_or(address);
    address.contains(':') && address.parse::<std::net::Ipv6Addr>().is_ok()
}

fn is_host(token: &str) -> bool {
    let name = token.split(':').next().unwrap_or(token);
    let labels: Vec<&str> = name.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|ending| HOST_ENDINGS.contains(ending) && !NAME_ENDINGS.contains(ending))
}

fn scrub_token(token: &str) -> Option<&'static str> {
    if token.contains("://") {
        return Some(URL);
    }
    if token.contains('@') && token.contains('.') && !token.starts_with('@') {
        return Some(EMAIL);
    }
    if is_mac(token) || is_ipv4(token) || is_ipv6(token) {
        return Some(ADDRESS);
    }
    if is_host(token) {
        return Some(HOST);
    }
    if token.starts_with("~/") {
        return Some(PATH);
    }
    if let Some(at) = token.find('/') {
        let path = &token[at..];
        let own =
            token.starts_with("crates/") || OWN_PATHS.iter().any(|root| path.starts_with(root));
        let relative_file = at > 0 && (token.contains('.') || token.matches('/').count() > 1);
        if !own && (at == 0 || relative_file) {
            return Some(PATH);
        }
    }
    None
}

fn scrub_quotes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(['"', '\'']) {
        let quote = rest[at..].chars().next().unwrap_or('"');
        let is_apostrophe = quote == '\'' && rest[..at].bytes().next_back().is_some_and(is_word);
        out.push_str(&rest[..=at]);
        let after = &rest[at + 1..];
        if is_apostrophe {
            rest = after;
            continue;
        }
        let Some(end) = after.find(quote) else {
            out.push_str(QUOTED);
            return out;
        };
        let inner = &after[..end];
        let identifier = inner.len() <= LONGEST_QUOTED
            && inner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b));
        out.push_str(if identifier { inner } else { QUOTED });
        out.push(quote);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

fn is_edge(character: char) -> bool {
    !character.is_alphanumeric() && !"/~[]_-.".contains(character)
}

impl Scrubber {
    pub fn with_secrets(secrets: impl IntoIterator<Item = String>) -> Self {
        let unique: BTreeSet<String> = secrets
            .into_iter()
            .map(|secret| secret.trim_matches(['\0', '\n', ' ', ',']).to_owned())
            .filter(|secret| secret.len() >= SHORTEST_SECRET)
            .collect();
        let mut secrets: Vec<String> = unique.into_iter().collect();
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Self { secrets }
    }

    pub fn for_this_computer() -> Self {
        let mut secrets = accounts();
        secrets.extend(network_names());
        secrets.extend(fs::read_to_string(HOSTNAME_FILE));
        secrets.extend(SERIAL_FILES.iter().filter_map(|file| {
            fs::read(file)
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        }));
        Self::with_secrets(secrets)
    }

    pub fn line(&self, line: &str) -> String {
        let mut text = scrub_quotes(line);
        for secret in &self.secrets {
            text = replace_words(&text, secret);
        }
        let mut out = String::with_capacity(text.len());
        let mut token = String::new();
        for character in text.chars() {
            if character.is_whitespace()
                || is_edge(character) && character != ':' && character != '@'
            {
                self.push_token(&mut out, &token);
                token.clear();
                out.push(character);
            } else {
                token.push(character);
            }
        }
        self.push_token(&mut out, &token);
        out
    }

    fn push_token(&self, out: &mut String, token: &str) {
        let trimmed = token.trim_end_matches([':', '.', '-']);
        match scrub_token(trimmed) {
            Some(replacement) => {
                out.push_str(replacement);
                out.push_str(&token[trimmed.len()..]);
            }
            None => out.push_str(token),
        }
    }

    pub fn text(&self, text: &str) -> String {
        let mut out: String = text
            .lines()
            .map(|line| self.line(line))
            .collect::<Vec<_>>()
            .join("\n");
        if text.ends_with('\n') {
            out.push('\n');
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Scrubber;

    #[test]
    fn nothing_personal_survives_and_diagnostics_do() {
        let scrubber = Scrubber::with_secrets(
            ["mikael", "HomeWifi", "minimac", "C02XYZ123"]
                .into_iter()
                .map(str::to_owned),
        );
        for (line, gone) in [
            ("cannot open /home/mikael/Documents/budget.ods", "budget"),
            ("failed: \"Letter to the bank.pdf\"", "Letter"),
            ("joined HomeWifi at 192.168.5.20", "192.168"),
            ("joined HomeWifi at 192.168.5.20", "HomeWifi"),
            ("lost smb://nas.lan/share/photos", "photos"),
            ("server nas.lan went away", "nas.lan"),
            ("host fe80::1234:5678:9abc:def0 down", "fe80"),
            ("station aa:bb:cc:dd:ee:ff left", "aa:bb"),
            ("serial C02XYZ123 on minimac", "C02XYZ123"),
            ("serial C02XYZ123 on minimac", "minimac"),
            ("mail from mikael@example.com", "example"),
            ("~/secret.txt missing", "secret"),
        ] {
            assert!(!scrubber.line(line).contains(gone), "{line}");
        }
        for line in [
            "bande-shell: Surface is not configured at 16:03:08",
            "/usr/lib/aarch64-linux-gnu/libc.so.6 + 0x81234",
            "panicked at crates/shell/shell/src/bar.rs:10:5",
            "output \"HDMI-A-1\" failed with -110",
            "bande_shell::bar::draw at bande-compositor.service",
            "[drm:apple_drm] *ERROR* flip_done timed out",
        ] {
            assert_eq!(scrubber.line(line), line);
        }
    }
}
