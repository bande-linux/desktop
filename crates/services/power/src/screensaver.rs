use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::message::Header;
use zbus::names::BusName;

pub const BUS_NAME: &str = "org.freedesktop.ScreenSaver";
pub const OBJECT_PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];

type Hold = Box<dyn Fn(&str, &str) -> Option<OwnedFd> + Send + Sync>;
type Report = Box<dyn Fn(bool) + Send + Sync>;

struct Inhibitor {
    owner: String,
    _held: Option<OwnedFd>,
}

#[derive(Default)]
struct Inhibitors {
    last_cookie: u32,
    active: BTreeMap<u32, Inhibitor>,
}

impl Inhibitors {
    fn fresh_cookie(&mut self) -> u32 {
        loop {
            self.last_cookie = self.last_cookie.wrapping_add(1);
            if self.last_cookie != 0 && !self.active.contains_key(&self.last_cookie) {
                return self.last_cookie;
            }
        }
    }
}

struct Registry {
    inhibitors: Mutex<Inhibitors>,
    report: Report,
}

impl Registry {
    fn change<T>(&self, edit: impl FnOnce(&mut Inhibitors) -> T) -> T {
        let mut inhibitors = self
            .inhibitors
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let outcome = edit(&mut inhibitors);
        (self.report)(!inhibitors.active.is_empty());
        outcome
    }
}

pub struct InhibitionService {
    _connection: Connection,
}

impl InhibitionService {
    pub fn start(
        connection: &Connection,
        hold: impl Fn(&str, &str) -> Option<OwnedFd> + Send + Sync + 'static,
        inhibited: impl Fn(bool) + Send + Sync + 'static,
    ) -> zbus::Result<Self> {
        let registry = Arc::new(Registry {
            inhibitors: Mutex::default(),
            report: Box::new(inhibited),
        });
        let owners = DBusProxy::new(connection)?.receive_name_owner_changed()?;
        let released = Arc::clone(&registry);
        thread::spawn(move || {
            for change in owners {
                let Ok(arguments) = change.args() else {
                    continue;
                };
                if arguments.new_owner().is_none() {
                    let gone = arguments.name().as_str();
                    released.change(|inhibitors| {
                        inhibitors
                            .active
                            .retain(|_, inhibitor| inhibitor.owner != gone)
                    });
                }
            }
        });
        let hold: Arc<Hold> = Arc::new(Box::new(hold));
        for path in OBJECT_PATHS {
            connection.object_server().at(
                path,
                ScreenSaver {
                    registry: Arc::clone(&registry),
                    hold: Arc::clone(&hold),
                },
            )?;
        }
        connection.request_name(BUS_NAME)?;
        Ok(Self {
            _connection: connection.clone(),
        })
    }
}

fn sender(header: &Header<'_>) -> zbus::fdo::Result<String> {
    header
        .sender()
        .map(ToString::to_string)
        .ok_or_else(|| zbus::fdo::Error::Failed("the call has no sender".to_owned()))
}

struct ScreenSaver {
    registry: Arc<Registry>,
    hold: Arc<Hold>,
}

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    async fn inhibit(
        &self,
        application_name: String,
        reason_for_inhibit: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<u32> {
        let owner = sender(&header)?;
        let held = (self.hold)(&application_name, &reason_for_inhibit);
        let cookie = self.registry.change(|inhibitors| {
            let cookie = inhibitors.fresh_cookie();
            inhibitors.active.insert(
                cookie,
                Inhibitor {
                    owner: owner.clone(),
                    _held: held,
                },
            );
            cookie
        });
        let present = zbus::fdo::DBusProxy::new(connection)
            .await?
            .name_has_owner(BusName::try_from(owner.as_str()).map_err(zbus::Error::from)?)
            .await?;
        if !present {
            self.registry.change(|inhibitors| {
                inhibitors.active.remove(&cookie);
            });
        }
        Ok(cookie)
    }

    fn un_inhibit(&self, cookie: u32, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        let caller = sender(&header)?;
        self.registry.change(|inhibitors| {
            if inhibitors
                .active
                .get(&cookie)
                .is_some_and(|inhibitor| inhibitor.owner == caller)
            {
                inhibitors.active.remove(&cookie);
            }
        });
        Ok(())
    }
}
