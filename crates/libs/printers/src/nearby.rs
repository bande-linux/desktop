use std::collections::HashMap;

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedObjectPath;
use zbus::{MatchRule, message};

const AVAHI: &str = "org.freedesktop.Avahi";
const BROWSER: &str = "org.freedesktop.Avahi.ServiceBrowser";
const UNSPECIFIED: i32 = -1;
const INET: i32 = 0;
const SERVICES: [(&str, &str); 2] = [("_ipp._tcp", "ipp"), ("_ipps._tcp", "ipps")];

#[zbus::proxy(
    interface = "org.freedesktop.Avahi.Server2",
    default_service = "org.freedesktop.Avahi",
    default_path = "/",
    gen_async = false,
    blocking_name = "AvahiProxy"
)]
trait Avahi {
    fn service_browser_prepare(
        &self,
        interface: i32,
        protocol: i32,
        kind: &str,
        domain: &str,
        flags: u32,
    ) -> zbus::Result<OwnedObjectPath>;

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    fn resolve_service(
        &self,
        interface: i32,
        protocol: i32,
        name: &str,
        kind: &str,
        domain: &str,
        address_protocol: i32,
        flags: u32,
    ) -> zbus::Result<(
        i32,
        i32,
        String,
        String,
        String,
        String,
        i32,
        String,
        u16,
        Vec<Vec<u8>>,
        u32,
    )>;
}

#[zbus::proxy(
    interface = "org.freedesktop.Avahi.ServiceBrowser",
    default_service = "org.freedesktop.Avahi",
    gen_async = false,
    blocking_name = "BrowserProxy"
)]
trait Browser {
    fn start(&self) -> zbus::Result<()>;
    fn free(&self) -> zbus::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyPrinter {
    pub name: String,
    pub model: String,
    pub location: String,
    pub uri: String,
}

fn text_records(records: &[Vec<u8>]) -> HashMap<String, String> {
    records
        .iter()
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let (key, value) = record.split_once('=')?;
            Some((key.to_ascii_lowercase(), value.to_owned()))
        })
        .collect()
}

fn resolve(avahi: &AvahiProxy<'_>, item: &Item, scheme: &str) -> Option<NearbyPrinter> {
    let (_, _, name, _, _, _, _, address, port, records, _) = avahi
        .resolve_service(
            item.interface,
            item.protocol,
            &item.name,
            &item.kind,
            &item.domain,
            INET,
            0,
        )
        .ok()?;
    let records = text_records(&records);
    let resource = records.get("rp").map_or("ipp/print", String::as_str);
    Some(NearbyPrinter {
        model: records.get("ty").cloned().unwrap_or_default(),
        location: records.get("note").cloned().unwrap_or_default(),
        uri: format!(
            "{scheme}://{address}:{port}/{}",
            resource.trim_start_matches('/')
        ),
        name,
    })
}

#[derive(Debug, Clone)]
struct Item {
    interface: i32,
    protocol: i32,
    name: String,
    kind: String,
    domain: String,
}

pub fn browse(
    system: &Connection,
    mut found: impl FnMut(Vec<NearbyPrinter>) -> bool,
) -> zbus::Result<()> {
    let avahi = AvahiProxy::new(system)?;
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(AVAHI)?
        .interface(BROWSER)?
        .build();
    let signals = MessageIterator::for_match_rule(rule, system, None)?;
    let mut browsers = Vec::new();
    for (kind, scheme) in SERVICES {
        let path = avahi.service_browser_prepare(UNSPECIFIED, INET, kind, "", 0)?;
        let browser = BrowserProxy::builder(system).path(path.clone())?.build()?;
        browsers.push((path, scheme, browser));
    }
    for (_, _, browser) in &browsers {
        browser.start()?;
    }
    let mut printers: Vec<(String, NearbyPrinter)> = Vec::new();
    for message in signals.flatten() {
        let header = message.header();
        let Some(scheme) = header.path().and_then(|path| {
            browsers
                .iter()
                .find(|(browser, _, _)| browser.as_str() == path.as_str())
                .map(|(_, scheme, _)| *scheme)
        }) else {
            continue;
        };
        let member = header.member().map(|member| member.to_string());
        let changed = match member.as_deref() {
            Some("ItemNew") => {
                let Ok((interface, protocol, name, kind, domain, _)) = message
                    .body()
                    .deserialize::<(i32, i32, String, String, String, u32)>()
                else {
                    continue;
                };
                let item = Item {
                    interface,
                    protocol,
                    name,
                    kind,
                    domain,
                };
                let known = printers
                    .iter()
                    .any(|(seen, printer)| *seen == item.name && printer.uri.starts_with("ipp://"));
                match resolve(&avahi, &item, scheme) {
                    Some(printer) if !known => {
                        printers.retain(|(seen, _)| *seen != item.name);
                        printers.push((item.name, printer));
                        true
                    }
                    _ => false,
                }
            }
            Some("ItemRemove") => {
                let Ok((_, _, name, _, _, _)) =
                    message
                        .body()
                        .deserialize::<(i32, i32, String, String, String, u32)>()
                else {
                    continue;
                };
                let before = printers.len();
                let prefix = format!("{scheme}://");
                printers
                    .retain(|(seen, printer)| *seen != name || !printer.uri.starts_with(&prefix));
                printers.len() != before
            }
            Some("AllForNow") => true,
            _ => false,
        };
        if changed {
            let mut list: Vec<NearbyPrinter> = printers
                .iter()
                .map(|(_, printer)| printer.clone())
                .collect();
            list.sort_by_key(|printer| printer.name.to_lowercase());
            if !found(list) {
                break;
            }
        }
    }
    for (_, _, browser) in browsers {
        let _ = browser.free();
    }
    Ok(())
}
