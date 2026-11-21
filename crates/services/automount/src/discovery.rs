use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::names::BusName;
use zbus::zvariant::OwnedObjectPath;
use zbus::{MatchRule, message};

const AVAHI: &str = "org.freedesktop.Avahi";
const SERVICE_BROWSER: &str = "org.freedesktop.Avahi.ServiceBrowser";
const ANY_INTERFACE: i32 = -1;
const ANY_PROTOCOL: i32 = -1;
const DOMAIN: &str = "local";
const FROM_THIS_COMPUTER: u32 = 0x8 | 0x10;
const PACKAGES: [&str; 2] = ["avahi-daemon", "libnss-mdns"];
const SERVICES: [(&str, &str); 5] = [
    ("_smb._tcp", "smb"),
    ("_nfs._tcp", "nfs"),
    ("_sftp-ssh._tcp", "sftp"),
    ("_webdav._tcp", "dav"),
    ("_webdavs._tcp", "davs"),
];

type Resolved = (
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
);

#[zbus::proxy(
    interface = "org.freedesktop.Avahi.Server2",
    default_service = "org.freedesktop.Avahi",
    default_path = "/"
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

    #[allow(clippy::too_many_arguments)]
    fn resolve_service(
        &self,
        interface: i32,
        protocol: i32,
        name: &str,
        kind: &str,
        domain: &str,
        address_protocol: i32,
        flags: u32,
    ) -> zbus::Result<Resolved>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nearby {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Availability {
    #[default]
    Ready,
    Missing,
    Installing,
}

impl Availability {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::Missing => "missing",
            Self::Installing => "installing",
        }
    }
}

fn preference(url: &str) -> usize {
    SERVICES
        .iter()
        .position(|(_, scheme)| url.starts_with(&format!("{scheme}://")))
        .unwrap_or(SERVICES.len())
}

#[derive(Clone)]
struct Item {
    interface: i32,
    protocol: i32,
    kind: String,
    name: String,
}

impl Item {
    fn same_service(&self, other: &Self) -> bool {
        self.kind == other.kind && self.name == other.name
    }

    fn same(&self, other: &Self) -> bool {
        self.same_service(other)
            && self.interface == other.interface
            && self.protocol == other.protocol
    }
}

fn url_of(scheme: &str, host: &str, port: u16) -> String {
    let default = matches!(
        (scheme, port),
        ("smb", 445) | ("sftp", 22) | ("dav", 80) | ("davs", 443) | ("nfs", 2049)
    );
    match default {
        true => format!("{scheme}://{host}"),
        false => match host.split_once('/') {
            Some((name, path)) => format!("{scheme}://{name}:{port}/{path}"),
            None => format!("{scheme}://{host}:{port}"),
        },
    }
}

fn locate(avahi: &AvahiProxyBlocking<'_>, item: &Item, scheme: &str) -> zbus::Result<Nearby> {
    let (.., host, _, _, port, txt, _) = avahi.resolve_service(
        item.interface,
        item.protocol,
        &item.name,
        &item.kind,
        DOMAIN,
        ANY_PROTOCOL,
        0,
    )?;
    let path = txt
        .iter()
        .find_map(|record| record.strip_prefix(b"path="))
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    Ok(Nearby {
        name: item.name.clone(),
        url: url_of(
            scheme,
            &format!("{}{path}", host.trim_end_matches('.')),
            port,
        ),
    })
}

fn avahi_installed(bus: &DBusProxy<'_>) -> zbus::Result<bool> {
    Ok(bus.name_has_owner(BusName::try_from(AVAHI)?)?
        || bus
            .list_activatable_names()?
            .iter()
            .any(|name| name.as_str() == AVAHI))
}

#[derive(Default)]
struct State {
    found: Vec<(Item, Nearby)>,
    unresolved: Vec<Item>,
    browsers: Vec<(OwnedObjectPath, &'static str)>,
    watchers: HashSet<String>,
    generation: u64,
    system: Option<Connection>,
    browsing: bool,
    missing: bool,
    installing: bool,
}

impl State {
    fn availability(&self) -> Availability {
        match (self.installing, self.missing) {
            (true, _) => Availability::Installing,
            (false, true) => Availability::Missing,
            (false, false) => Availability::Ready,
        }
    }

    fn list(&self) -> Vec<Nearby> {
        let mut list: Vec<Nearby> = self.found.iter().map(|(_, entry)| entry.clone()).collect();
        list.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then_with(|| preference(&a.url).cmp(&preference(&b.url)))
        });
        list.dedup_by(|a, b| a.name == b.name);
        list
    }
}

#[derive(Clone)]
pub struct Discovery {
    state: Arc<Mutex<State>>,
    session: Connection,
    changed: Arc<dyn Fn() + Send + Sync>,
}

impl Discovery {
    pub fn new(session: Connection, changed: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::default(),
            session,
            changed: Arc::new(changed),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn watch(&self, watcher: String) {
        let mut state = self.lock();
        if !state.watchers.insert(watcher.clone()) {
            return;
        }
        let first = state.watchers.len() == 1;
        let generation = state.generation;
        drop(state);
        self.follow_watcher(watcher);
        if first && let Err(error) = self.start(generation) {
            tracing::warn!("cannot browse the network: {error}");
        }
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
            self.lock().watchers.clear();
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
        let mut state = self.lock();
        if !state.watchers.remove(watcher) || !state.watchers.is_empty() {
            return;
        }
        state.generation += 1;
        state.browsers.clear();
        state.browsing = false;
        let system = state.system.take();
        let had = !state.found.is_empty() || state.missing;
        state.found.clear();
        state.unresolved.clear();
        state.missing = false;
        drop(state);
        if let Some(system) = system
            && let Err(error) = system.close()
        {
            tracing::debug!("network browsing already stopped: {error}");
        }
        if had {
            (self.changed)();
        }
    }

    fn start(&self, generation: u64) -> zbus::Result<()> {
        let system = bande_bus::system()?;
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(AVAHI)?
            .interface(SERVICE_BROWSER)?
            .build();
        let items = MessageIterator::for_match_rule(rule, &system, None)?;
        let bus = DBusProxy::new(&system)?;
        let owners = bus.receive_name_owner_changed_with_args(&[(0, AVAHI)])?;
        {
            let mut state = self.lock();
            if state.generation != generation {
                return Ok(());
            }
            state.system = Some(system.clone());
        }
        let avahi = AvahiProxyBlocking::new(&system)?;
        let (resolve, to_resolve) = mpsc::channel();
        self.spawn("network resolver", move |discovery| {
            for (item, scheme) in to_resolve {
                discovery.resolve(generation, &avahi, item, scheme);
            }
        })?;
        self.spawn("network browser", move |discovery| {
            discovery.follow_items(generation, items, &resolve);
        })?;
        self.refresh(generation, &system);
        self.spawn("avahi watcher", move |discovery| {
            for change in owners {
                let Ok(args) = change.args() else {
                    continue;
                };
                discovery.forget_browsers(generation);
                if args.new_owner().is_some() {
                    discovery.refresh(generation, &system);
                }
            }
        })
    }

    fn refresh(&self, generation: u64, system: &Connection) {
        match DBusProxy::new(system).and_then(|bus| avahi_installed(&bus)) {
            Ok(installed) => self.update(generation, |state| state.missing = !installed),
            Err(error) => tracing::warn!("cannot find Avahi: {error}"),
        }
        if !self.lock().missing
            && let Err(error) = self.browse(generation, system)
        {
            self.lock().browsing = false;
            tracing::warn!("cannot browse the network: {error}");
        }
    }

    fn spawn(&self, name: &str, work: impl FnOnce(Discovery) + Send + 'static) -> zbus::Result<()> {
        let discovery = self.clone();
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || work(discovery))
            .map(drop)
            .map_err(|error| zbus::Error::Failure(error.to_string()))
    }

    fn browse(&self, generation: u64, system: &Connection) -> zbus::Result<()> {
        {
            let mut state = self.lock();
            if state.generation != generation || state.browsing {
                return Ok(());
            }
            state.browsing = true;
        }
        let avahi = AvahiProxyBlocking::new(system)?;
        for (kind, scheme) in SERVICES {
            let path =
                avahi.service_browser_prepare(ANY_INTERFACE, ANY_PROTOCOL, kind, DOMAIN, 0)?;
            {
                let mut state = self.lock();
                if state.generation != generation {
                    return Ok(());
                }
                state.browsers.push((path.clone(), scheme));
            }
            system.call_method(Some(AVAHI), &path, Some(SERVICE_BROWSER), "Start", &())?;
        }
        Ok(())
    }

    fn follow_items(
        &self,
        generation: u64,
        items: MessageIterator,
        resolve: &Sender<(Item, &'static str)>,
    ) {
        for message in items.flatten() {
            let header = message.header();
            let (Some(path), Some(member)) = (header.path(), header.member()) else {
                continue;
            };
            let scheme = {
                let state = self.lock();
                if state.generation != generation {
                    return;
                }
                state
                    .browsers
                    .iter()
                    .find(|(browser, _)| browser.as_ref() == *path)
                    .map(|(_, scheme)| *scheme)
            };
            let Some(scheme) = scheme else {
                continue;
            };
            if member.as_str() == "Failure" {
                let error: String = message.body().deserialize().unwrap_or_default();
                tracing::warn!("cannot browse the network for {scheme}: {error}");
                continue;
            }
            let Ok((interface, protocol, name, kind, _, flags)) =
                message
                    .body()
                    .deserialize::<(i32, i32, String, String, String, u32)>()
            else {
                continue;
            };
            let item = Item {
                interface,
                protocol,
                kind,
                name,
            };
            match member.as_str() {
                "ItemNew" if flags & FROM_THIS_COMPUTER == 0 => {
                    self.add(generation, item, scheme, resolve);
                }
                "ItemRemove" => self.update(generation, |state| {
                    state.found.retain(|(known, _)| !known.same(&item));
                    state.unresolved.retain(|known| !known.same(&item));
                }),
                _ => {}
            }
        }
    }

    fn add(
        &self,
        generation: u64,
        item: Item,
        scheme: &'static str,
        resolve: &Sender<(Item, &'static str)>,
    ) {
        let mut wanted = false;
        self.update(generation, |state| {
            if state.found.iter().any(|(known, _)| known.same(&item))
                || state.unresolved.iter().any(|known| known.same(&item))
            {
                return;
            }
            match state
                .found
                .iter()
                .find(|(known, _)| known.same_service(&item))
            {
                Some((_, entry)) => {
                    let entry = entry.clone();
                    state.found.push((item.clone(), entry));
                }
                None => {
                    state.unresolved.push(item.clone());
                    wanted = true;
                }
            }
        });
        if wanted && resolve.send((item, scheme)).is_err() {
            tracing::debug!("network browsing already stopped");
        }
    }

    fn resolve(&self, generation: u64, avahi: &AvahiProxyBlocking<'_>, item: Item, scheme: &str) {
        if !self.lock().unresolved.iter().any(|known| known.same(&item)) {
            return;
        }
        let located = locate(avahi, &item, scheme)
            .inspect_err(|error| {
                tracing::debug!("cannot resolve {} {}: {error}", item.kind, item.name);
            })
            .ok();
        self.update(generation, |state| {
            let before = state.unresolved.len();
            state.unresolved.retain(|known| !known.same(&item));
            if let Some(entry) = located
                && state.unresolved.len() < before
            {
                state.found.push((item, entry));
            }
        });
    }

    fn forget_browsers(&self, generation: u64) {
        self.update(generation, |state| {
            state.browsers.clear();
            state.browsing = false;
            state.found.clear();
            state.unresolved.clear();
        });
    }

    fn update(&self, generation: u64, change: impl FnOnce(&mut State)) {
        let mut state = self.lock();
        if state.generation != generation {
            return;
        }
        let before = (state.list(), state.availability());
        change(&mut state);
        let changed = (state.list(), state.availability()) != before;
        drop(state);
        if changed {
            (self.changed)();
        }
    }

    pub fn list(&self) -> Vec<Nearby> {
        self.lock().list()
    }

    pub fn availability(&self) -> Availability {
        self.lock().availability()
    }

    pub fn install(&self, install: impl FnOnce(&[String]) -> Result<(), String>) {
        let generation = {
            let mut state = self.lock();
            if state.installing {
                return;
            }
            state.installing = true;
            state.generation
        };
        (self.changed)();
        let packages: Vec<String> = PACKAGES.iter().map(|name| (*name).to_owned()).collect();
        if let Err(error) = install(&packages) {
            tracing::warn!("cannot install {}: {error}", packages.join(", "));
        }
        self.lock().installing = false;
        (self.changed)();
        let system = self.lock().system.clone();
        if let Some(system) = system {
            self.refresh(generation, &system);
        }
    }
}
