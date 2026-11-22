use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use bande_discovery::domain_of;

use crate::http;
use crate::net::{Network, Security, Server};

const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
const CONFIG_LIMIT: usize = 1 << 20;
const PASSWORD_METHODS: [&str; 2] = ["password-cleartext", "plain"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub imap: Server,
    pub smtp: Server,
    pub user: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Services,
    Domain,
    Guess,
}

fn expand(template: &str, email: &str) -> String {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    template
        .replace("%EMAILADDRESS%", email)
        .replace("%EMAILLOCALPART%", local)
        .replace("%EMAILDOMAIN%", domain)
}

fn security_of(socket: &str) -> Option<Security> {
    match socket.to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Some(Security::Tls),
        "STARTTLS" => Some(Security::StartTls),
        _ => None,
    }
}

fn rank(security: Security) -> u8 {
    match security {
        Security::Tls => 0,
        Security::StartTls => 1,
        Security::Plain => 2,
    }
}

pub fn parse_config(xml: &str, email: &str) -> Option<Settings> {
    let document = roxmltree::Document::parse(xml).ok()?;
    let provider = document
        .descendants()
        .find(|node| node.has_tag_name("emailProvider"))?;
    let child_text = |node: roxmltree::Node<'_, '_>, name: &str| {
        node.children()
            .find(|child| child.has_tag_name(name))
            .and_then(|child| child.text())
            .map(|text| text.trim().to_owned())
    };
    let takes_password = |node: roxmltree::Node<'_, '_>| {
        let mut methods = node
            .children()
            .filter(|child| child.has_tag_name("authentication"))
            .filter_map(|child| child.text())
            .peekable();
        methods.peek().is_none() || methods.any(|method| PASSWORD_METHODS.contains(&method.trim()))
    };
    let servers = |tag: &str, kind: &str| {
        let mut found = provider
            .children()
            .filter(|node| node.has_tag_name(tag) && node.attribute("type") == Some(kind))
            .filter(|node| takes_password(*node))
            .filter_map(|node| {
                let security = security_of(&child_text(node, "socketType")?)?;
                let server = Server {
                    host: expand(&child_text(node, "hostname")?, email).to_ascii_lowercase(),
                    port: child_text(node, "port")?.parse().ok()?,
                    security,
                };
                let user = expand(
                    &child_text(node, "username").unwrap_or_else(|| "%EMAILADDRESS%".to_owned()),
                    email,
                );
                Some((server, user))
            })
            .collect::<Vec<_>>();
        found.sort_by_key(|(server, _)| rank(server.security));
        found.into_iter().next()
    };
    let (imap, user) = servers("incomingServer", "imap")?;
    let (smtp, _) = servers("outgoingServer", "smtp")?;
    Some(Settings { imap, smtp, user })
}

fn from_config(url: &str, email: &str, network: &Network) -> Option<Settings> {
    let body = http::get(url, http::XML, network, CONFIG_LIMIT)
        .inspect_err(|error| tracing::debug!("{url}: {error}"))
        .ok()?;
    parse_config(&String::from_utf8_lossy(&body), email)
}

fn query_value(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'@' | b'.' | b'-' | b'_' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn from_services(domain: &str, email: &str, network: &Network) -> Option<Settings> {
    let first = |names: [(&str, Security); 2]| {
        names.into_iter().find_map(|(service, security)| {
            network
                .resolver
                .services(&format!("{service}.{domain}"))
                .into_iter()
                .next()
                .map(|target| Server {
                    host: target.host,
                    port: target.port,
                    security,
                })
        })
    };
    let imap = first([
        ("_imaps._tcp", Security::Tls),
        ("_imap._tcp", Security::StartTls),
    ])?;
    let smtp = first([
        ("_submissions._tcp", Security::Tls),
        ("_submission._tcp", Security::StartTls),
    ])?;
    Some(Settings {
        imap,
        smtp,
        user: email.to_owned(),
    })
}

fn answers(host: &str, port: u16) -> bool {
    (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .is_some_and(|address| TcpStream::connect_timeout(&address, PROBE_TIMEOUT).is_ok())
}

fn guess(domain: &str, email: &str) -> Option<Settings> {
    let probe = |candidates: &[(&str, u16, Security)]| {
        candidates.iter().find_map(|(prefix, port, security)| {
            let host = format!("{prefix}.{domain}");
            answers(&host, *port).then_some(Server {
                host,
                port: *port,
                security: *security,
            })
        })
    };
    let imap = probe(&[
        ("imap", 993, Security::Tls),
        ("mail", 993, Security::Tls),
        ("imap", 143, Security::StartTls),
        ("mail", 143, Security::StartTls),
    ])?;
    let smtp = probe(&[
        ("smtp", 465, Security::Tls),
        ("mail", 465, Security::Tls),
        ("smtp", 587, Security::StartTls),
        ("mail", 587, Security::StartTls),
    ])?;
    Some(Settings {
        imap,
        smtp,
        user: email.to_owned(),
    })
}

pub fn discover(email: &str, network: &Network) -> Option<(Settings, Source)> {
    let email = email.trim();
    let domain = domain_of(email)?;
    if let Some(found) = from_services(&domain, email, network) {
        return Some((found, Source::Services));
    }
    let published = [
        format!(
            "https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={}",
            query_value(email)
        ),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ];
    if let Some(found) = published
        .iter()
        .find_map(|url| from_config(url, email, network))
    {
        return Some((found, Source::Domain));
    }
    guess(&domain, email).map(|found| (found, Source::Guess))
}
