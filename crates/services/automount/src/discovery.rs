use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_discovery::nearby::{Addresses, Availability, Browser, Service};
use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::names::BusName;

const SERVICES: [(&str, &str); 6] = [
    ("_smb._tcp", "smb"),
    ("_nfs._tcp", "nfs"),
    ("_sftp-ssh._tcp", "sftp"),
    ("_webdav._tcp", "dav"),
    ("_webdavs._tcp", "davs"),
    (SCREEN_KIND, "vnc"),
];
const SCREEN_KIND: &str = "_rfb._tcp";

/// One computer on the network: its files (or its screen, when it shares only that) at `url`,
/// and its shared screen, if it shares files too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nearby {
    pub name: String,
    pub url: String,
    pub screen: Option<String>,
}

fn preference(url: &str) -> usize {
    SERVICES
        .iter()
        .position(|(_, scheme)| url.starts_with(&format!("{scheme}://")))
        .unwrap_or(SERVICES.len())
}

fn plain_host(host: &str) -> bool {
    !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_'))
}

fn url_of(scheme: &str, host: &str, port: u16, path: &str) -> String {
    let path = match path.starts_with('/') {
        true => bande_xdg::percent_encode(Path::new(path)),
        false => String::new(),
    };
    let default = matches!(
        (scheme, port),
        ("smb", 445) | ("sftp", 22) | ("dav", 80) | ("davs", 443) | ("nfs", 2049) | ("vnc", 5900)
    );
    match default {
        true => format!("{scheme}://{host}{path}"),
        false => format!("{scheme}://{host}:{port}{path}"),
    }
}

#[derive(Clone)]
pub struct Discovery {
    browser: Browser,
    watchers: Arc<Mutex<HashSet<String>>>,
    session: Connection,
}

impl Discovery {
    pub fn new(session: Connection, changed: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            browser: Browser::new(&SERVICES.map(|(kind, _)| kind), Addresses::Any, changed),
            watchers: Arc::default(),
            session,
        }
    }

    fn watchers(&self) -> MutexGuard<'_, HashSet<String>> {
        self.watchers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn watch(&self, watcher: String) {
        let mut watchers = self.watchers();
        if !watchers.insert(watcher.clone()) {
            return;
        }
        if watchers.len() == 1 {
            self.browser.start();
        }
        drop(watchers);
        self.follow_watcher(watcher);
    }

    fn follow_watcher(&self, watcher: String) {
        let discovery = self.clone();
        let spawned = thread::Builder::new()
            .name("network watcher".to_owned())
            .spawn(move || {
                if let Err(error) = discovery.wait_until_gone(&watcher) {
                    tracing::warn!("cannot follow {watcher}: {error}");
                }
                discovery.unwatch(&watcher);
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot follow a network watcher: {error}");
            self.watchers().clear();
            self.browser.stop();
        }
    }

    fn wait_until_gone(&self, watcher: &str) -> zbus::Result<()> {
        let bus = DBusProxy::new(&self.session)?;
        let changes = bus.receive_name_owner_changed_with_args(&[(0, watcher)])?;
        let name = BusName::try_from(watcher)?;
        if !bus.name_has_owner(name)? {
            return Ok(());
        }
        for change in changes {
            if change.args().is_ok_and(|args| args.new_owner().is_none()) {
                break;
            }
        }
        Ok(())
    }

    fn unwatch(&self, watcher: &str) {
        let mut watchers = self.watchers();
        if watchers.remove(watcher) && watchers.is_empty() {
            self.browser.stop();
        }
    }

    pub fn list(&self) -> Vec<Nearby> {
        computers(self.browser.services())
    }

    pub fn availability(&self) -> Availability {
        self.browser.availability()
    }

    pub fn install(&self, install: impl FnOnce(&[String]) -> Result<(), String>) {
        self.browser.install(install);
    }
}

struct Found {
    name: String,
    url: String,
    host: String,
    address: String,
    screen: bool,
}

/// One entry per computer, matched by host name or address: its preferred file service, with
/// its screen beside it, or its screen alone.
pub fn computers(services: Vec<Service>) -> Vec<Nearby> {
    let mut found: Vec<Found> = services
        .into_iter()
        .filter_map(|service| {
            let (kind, scheme) = SERVICES.iter().find(|(kind, _)| *kind == service.kind)?;
            if !plain_host(&service.host) {
                tracing::debug!(
                    "ignored {} with the host name {:?}",
                    service.name,
                    service.host
                );
                return None;
            }
            let path = service.value("path").unwrap_or_default();
            Some(Found {
                url: url_of(scheme, &service.host, service.port, &path),
                host: service.host.to_lowercase(),
                address: service.address,
                screen: *kind == SCREEN_KIND,
                name: service.name,
            })
        })
        .collect();
    found.sort_by(|a, b| {
        preference(&a.url)
            .cmp(&preference(&b.url))
            .then_with(|| a.name.cmp(&b.name))
    });
    let same_computer = |a: &Found, b: &Found| {
        a.host == b.host || (!a.address.is_empty() && a.address == b.address)
    };
    let mut computers: Vec<(Found, Option<String>)> = Vec::new();
    for service in found {
        match computers
            .iter_mut()
            .find(|(computer, _)| same_computer(computer, &service))
        {
            Some((computer, screen)) if service.screen && !computer.screen => {
                screen.get_or_insert(service.url);
            }
            Some(_) => {}
            None => computers.push((service, None)),
        }
    }
    let mut list: Vec<Nearby> = computers
        .into_iter()
        .map(|(computer, screen)| Nearby {
            name: computer.name,
            url: computer.url,
            screen,
        })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.url.cmp(&b.url)));
    list
}
