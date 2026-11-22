use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Request, Signal};
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{MatchRule, message};

const BOLT: &str = "org.freedesktop.bolt";
const UNAUTHORIZED: &str = "connected";
const ALLOW: &str = "allow";
const ALWAYS_ALLOW: &str = "always-allow";

#[zbus::proxy(
    interface = "org.freedesktop.bolt1.Manager",
    default_service = "org.freedesktop.bolt",
    default_path = "/org/freedesktop/bolt"
)]
trait Manager {
    fn list_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(allow_interactive_auth)]
    fn enroll_device(&self, uid: &str, policy: &str, flags: &str) -> zbus::Result<OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.freedesktop.bolt1.Device",
    default_service = "org.freedesktop.bolt"
)]
trait Device {
    #[zbus(allow_interactive_auth)]
    fn authorize(&self, flags: &str) -> zbus::Result<()>;

    #[zbus(property)]
    fn uid(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn status(&self) -> zbus::Result<String>;
}

#[derive(Default)]
struct State {
    unlocked: bool,
    shown: HashMap<OwnedObjectPath, u32>,
}

#[derive(Clone)]
pub struct Thunderbolt {
    system: Connection,
    session: Connection,
    state: Arc<Mutex<State>>,
}

impl Thunderbolt {
    pub fn new(system: &Connection, session: &Connection) -> Self {
        Self {
            system: system.clone(),
            session: session.clone(),
            state: Arc::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn follow(&self) -> zbus::Result<()> {
        let rule = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(BOLT)?
            .build();
        let signals = MessageIterator::for_match_rule(rule, &self.system, None)?;
        let owners =
            DBusProxy::new(&self.system)?.receive_name_owner_changed_with_args(&[(0, BOLT)])?;
        let checker = self.clone();
        thread::Builder::new()
            .name("thunderbolt".to_owned())
            .spawn(move || {
                for signal in signals.flatten() {
                    let header = signal.header();
                    let changed = header.member().is_some_and(|member| {
                        member.as_str() == "DeviceAdded" || member.as_str() == "PropertiesChanged"
                    });
                    if changed {
                        checker.check();
                    }
                }
            })?;
        let restarted = self.clone();
        thread::Builder::new()
            .name("thunderbolt-owner".to_owned())
            .spawn(move || {
                for change in owners {
                    if change.args().is_ok_and(|args| args.new_owner().is_some()) {
                        restarted.check();
                    }
                }
            })?;
        Ok(())
    }

    pub fn session_changed(&self, unlocked: bool) {
        self.state().unlocked = unlocked;
        if unlocked {
            self.check();
        }
    }

    fn check(&self) {
        if !self.state().unlocked {
            return;
        }
        let waiting = ManagerProxyBlocking::new(&self.system).and_then(|manager| {
            let mut waiting = Vec::new();
            if !DBusProxy::new(&self.system)?.name_has_owner(BOLT.try_into()?)? {
                return Ok(waiting);
            }
            for path in manager.list_devices()? {
                let device = DeviceProxyBlocking::builder(&self.system)
                    .path(path.clone())?
                    .cache_properties(CacheProperties::No)
                    .build()?;
                if device.status()? == UNAUTHORIZED {
                    waiting.push((path, device.name()?));
                }
            }
            Ok(waiting)
        });
        match waiting {
            Ok(waiting) => {
                for (path, name) in waiting {
                    self.ask(path, &name);
                }
            }
            Err(error) => tracing::warn!("cannot read the Thunderbolt devices: {error}"),
        }
    }

    fn ask(&self, path: OwnedObjectPath, name: &str) {
        if self.state().shown.contains_key(&path) {
            return;
        }
        let allow = tr!("Allow Once");
        let always = tr!("Always Allow");
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &tr!("Thunderbolt"),
                replaces_id: 0,
                app_icon: "computer-symbolic",
                summary: &tr!("Allow “{name}”?", name = name),
                body: &tr!(
                    "A Thunderbolt device gets direct access to this computer. Allow it only if you know where it came from."
                ),
                actions: &[(ALLOW, &allow), (ALWAYS_ALLOW, &always)],
                urgency: Urgency::Normal,
                sound: None,
                desktop_entry: None,
            },
        );
        let id = match posted {
            Ok(id) => id,
            Err(error) => {
                tracing::warn!("cannot ask to allow {name}: {error}");
                return;
            }
        };
        self.state().shown.insert(path.clone(), id);
        let answer = self.clone();
        let spawned = thread::Builder::new()
            .name("thunderbolt-answer".to_owned())
            .spawn(move || {
                let mut chosen = None;
                let followed = client::listen(&answer.session, |signal| match signal {
                    Signal::ActionInvoked(shown, key) if shown == id => {
                        chosen = Some(key);
                        false
                    }
                    Signal::Closed(shown) => shown != id,
                    _ => true,
                });
                if let Err(error) = followed {
                    tracing::warn!("the Thunderbolt question cannot be answered: {error}");
                }
                if let Some(key) = chosen
                    && let Err(error) = answer.allow(&path, &key)
                {
                    tracing::warn!("cannot allow the Thunderbolt device: {error}");
                }
                answer.state().shown.remove(&path);
            });
        if let Err(error) = spawned {
            tracing::warn!("the Thunderbolt question cannot be answered: {error}");
        }
    }

    fn allow(&self, path: &OwnedObjectPath, key: &str) -> zbus::Result<()> {
        let device = DeviceProxyBlocking::builder(&self.system)
            .path(path.clone())?
            .build()?;
        match key {
            ALLOW => device.authorize(""),
            ALWAYS_ALLOW => {
                ManagerProxyBlocking::new(&self.system)?.enroll_device(
                    &device.uid()?,
                    "auto",
                    "",
                )?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
