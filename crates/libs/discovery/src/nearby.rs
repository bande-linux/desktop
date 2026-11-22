//! Services that computers on the local network announce over multicast DNS (RFC 6762, RFC 6763),
//! browsed through Avahi's D-Bus interface while something asks for them.

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
const IPV4: i32 = 0;
const DOMAIN: &str = "local";
const FROM_THIS_COMPUTER: u32 = 0x8 | 0x10;

/// The packages that browse the local network and resolve the `.local` names found on it.
pub const PACKAGES: [&str; 2] = ["avahi-daemon", "libnss-mdns"];

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

    fn resolve_host_name(
        &self,
        interface: i32,
        protocol: i32,
        name: &str,
        address_protocol: i32,
        flags: u32,
    ) -> zbus::Result<(i32, i32, String, i32, String, u32)>;
}

/// The address a `.local` host name resolves to now, asked of Avahi.
pub fn resolve_host(name: &str, addresses: Addresses) -> zbus::Result<String> {
    let system = bande_bus::system()?;
    let (.., address, _) = AvahiProxyBlocking::new(&system)?.resolve_host_name(
        ANY_INTERFACE,
        addresses.protocol(),
        name,
        addresses.protocol(),
        0,
    )?;
    Ok(address)
}

/// Which addresses services are browsed over and resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addresses {
    Any,
    Ipv4,
}

impl Addresses {
    fn protocol(self) -> i32 {
        match self {
            Self::Any => ANY_PROTOCOL,
            Self::Ipv4 => IPV4,
        }
    }
}

/// Whether Avahi is there to browse with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Availability {
    #[default]
    Ready,
    Missing,
    Installing,
}

impl Availability {
    /// The availability as sent over D-Bus: empty when ready.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Ready => "",
            Self::Missing => "missing",
            Self::Installing => "installing",
        }
    }
}

/// A service another computer on the local network announces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub kind: String,
    pub host: String,
    pub address: String,
    pub port: u16,
    pub text: Vec<Vec<u8>>,
}

impl Service {
    /// The value of the TXT record entry `key`, matched without regard to case (RFC 6763 §6.4).
    pub fn value(&self, key: &str) -> Option<String> {
        self.text.iter().find_map(|entry| {
            let (name, value) = entry.split_at(entry.iter().position(|byte| *byte == b'=')?);
            name.eq_ignore_ascii_case(key.as_bytes())
                .then(|| String::from_utf8_lossy(value.get(1..).unwrap_or_default()).into_owned())
        })
    }
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

fn avahi_installed(bus: &DBusProxy<'_>) -> zbus::Result<bool> {
    Ok(bus.name_has_owner(BusName::try_from(AVAHI)?)?
        || bus
            .list_activatable_names()?
            .iter()
            .any(|name| name.as_str() == AVAHI))
}

#[derive(Default)]
struct State {
    found: Vec<(Item, Service)>,
    unresolved: Vec<Item>,
    browsers: Vec<OwnedObjectPath>,
    generation: u64,
    system: Option<Connection>,
    running: bool,
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

    fn services(&self) -> Vec<Service> {
        let mut list: Vec<Service> = self.found.iter().map(|(_, entry)| entry.clone()).collect();
        list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.kind.cmp(&b.kind)));
        list.dedup_by(|a, b| a.name == b.name && a.kind == b.kind);
        list
    }
}

/// Browses Avahi for services of the given kinds, such as `_ipp._tcp`, between
/// [`start`](Self::start) and [`stop`](Self::stop), and follows Avahi coming and going.
#[derive(Clone)]
pub struct Browser {
    state: Arc<Mutex<State>>,
    kinds: Arc<[String]>,
    addresses: Addresses,
    changed: Arc<dyn Fn() + Send + Sync>,
}

impl Browser {
    /// A browser that calls `changed` whenever the services found or the availability change.
    pub fn new(
        kinds: &[&str],
        addresses: Addresses,
        changed: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            state: Arc::default(),
            kinds: kinds.iter().map(|kind| (*kind).to_owned()).collect(),
            addresses,
            changed: Arc::new(changed),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts browsing, unless it already is.
    pub fn start(&self) {
        let generation = {
            let mut state = self.lock();
            if state.running {
                return;
            }
            state.running = true;
            state.generation
        };
        if let Err(error) = self.follow(generation) {
            tracing::warn!("cannot browse the network: {error}");
            self.update(generation, |state| state.running = false);
        }
    }

    /// Stops browsing and forgets what was found.
    pub fn stop(&self) {
        let mut state = self.lock();
        if !state.running {
            return;
        }
        state.generation += 1;
        state.running = false;
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

    fn follow(&self, generation: u64) -> zbus::Result<()> {
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
        self.spawn("network resolver", move |browser| {
            for item in to_resolve {
                browser.resolve(generation, &avahi, item);
            }
        })?;
        self.spawn("network browser", move |browser| {
            browser.follow_items(generation, items, &resolve);
        })?;
        self.refresh(generation, &system);
        self.spawn("avahi watcher", move |browser| {
            for change in owners {
                let Ok(args) = change.args() else {
                    continue;
                };
                browser.forget_browsers(generation);
                if args.new_owner().is_some() {
                    browser.refresh(generation, &system);
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

    fn spawn(&self, name: &str, work: impl FnOnce(Browser) + Send + 'static) -> zbus::Result<()> {
        let browser = self.clone();
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || work(browser))
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
        for kind in self.kinds.iter() {
            let path = avahi.service_browser_prepare(
                ANY_INTERFACE,
                self.addresses.protocol(),
                kind,
                DOMAIN,
                0,
            )?;
            {
                let mut state = self.lock();
                if state.generation != generation {
                    return Ok(());
                }
                state.browsers.push(path.clone());
            }
            system.call_method(Some(AVAHI), &path, Some(SERVICE_BROWSER), "Start", &())?;
        }
        Ok(())
    }

    fn follow_items(&self, generation: u64, items: MessageIterator, resolve: &Sender<Item>) {
        for message in items.flatten() {
            let header = message.header();
            let (Some(path), Some(member)) = (header.path(), header.member()) else {
                continue;
            };
            {
                let state = self.lock();
                if state.generation != generation {
                    return;
                }
                if !state
                    .browsers
                    .iter()
                    .any(|browser| browser.as_ref() == *path)
                {
                    continue;
                }
            }
            if member.as_str() == "Failure" {
                let error: String = message.body().deserialize().unwrap_or_default();
                tracing::warn!("cannot browse the network: {error}");
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
                    self.add(generation, item, resolve);
                }
                "ItemRemove" => self.update(generation, |state| {
                    state.found.retain(|(known, _)| !known.same(&item));
                    state.unresolved.retain(|known| !known.same(&item));
                }),
                _ => {}
            }
        }
    }

    fn add(&self, generation: u64, item: Item, resolve: &Sender<Item>) {
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
        if wanted && resolve.send(item).is_err() {
            tracing::debug!("network browsing already stopped");
        }
    }

    fn locate(&self, avahi: &AvahiProxyBlocking<'_>, item: &Item) -> zbus::Result<Service> {
        let (.., host, _, address, port, text, _) = avahi.resolve_service(
            item.interface,
            item.protocol,
            &item.name,
            &item.kind,
            DOMAIN,
            self.addresses.protocol(),
            0,
        )?;
        Ok(Service {
            name: item.name.clone(),
            kind: item.kind.clone(),
            host: host.trim_end_matches('.').to_owned(),
            address,
            port,
            text,
        })
    }

    fn resolve(&self, generation: u64, avahi: &AvahiProxyBlocking<'_>, item: Item) {
        if !self.lock().unresolved.iter().any(|known| known.same(&item)) {
            return;
        }
        let located = self
            .locate(avahi, &item)
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
        let before = (state.services(), state.availability());
        change(&mut state);
        let changed = (state.services(), state.availability()) != before;
        drop(state);
        if changed {
            (self.changed)();
        }
    }

    /// The services found, one for each name and kind, sorted by name.
    pub fn services(&self) -> Vec<Service> {
        self.lock().services()
    }

    pub fn availability(&self) -> Availability {
        self.lock().availability()
    }

    /// Installs [`PACKAGES`] through `install`, showing [`Availability::Installing`] meanwhile,
    /// then looks for Avahi again.
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
