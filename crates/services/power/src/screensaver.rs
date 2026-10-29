use std::collections::BTreeMap;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::message::Header;
use zbus::names::BusName;

pub const BUS_NAME: &str = "org.freedesktop.ScreenSaver";
pub const OBJECT_PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];

type Hold = Box<dyn Fn(&str, &str) -> Option<OwnedFd> + Send + Sync>;

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

type Shared = Arc<Mutex<Inhibitors>>;

fn lock(inhibitors: &Shared) -> MutexGuard<'_, Inhibitors> {
    inhibitors.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct InhibitionService {
    _connection: Connection,
}

impl InhibitionService {
    pub fn start(
        connection: &Connection,
        hold: impl Fn(&str, &str) -> Option<OwnedFd> + Send + Sync + 'static,
    ) -> zbus::Result<Self> {
        let inhibitors = Shared::default();
        let owners = DBusProxy::new(connection)?.receive_name_owner_changed()?;
        let released = Arc::clone(&inhibitors);
        thread::spawn(move || {
            for change in owners {
                let Ok(arguments) = change.args() else {
                    continue;
                };
                if arguments.new_owner().is_none() {
                    let gone = arguments.name().as_str();
                    lock(&released)
                        .active
                        .retain(|_, inhibitor| inhibitor.owner != gone);
                }
            }
        });
        let hold: Arc<Hold> = Arc::new(Box::new(hold));
        for path in OBJECT_PATHS {
            connection.object_server().at(
                path,
                ScreenSaver {
                    inhibitors: Arc::clone(&inhibitors),
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
    inhibitors: Shared,
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
        let cookie = {
            let mut inhibitors = lock(&self.inhibitors);
            let cookie = inhibitors.fresh_cookie();
            inhibitors.active.insert(
                cookie,
                Inhibitor {
                    owner: owner.clone(),
                    _held: held,
                },
            );
            cookie
        };
        let present = zbus::fdo::DBusProxy::new(connection)
            .await?
            .name_has_owner(BusName::try_from(owner.as_str()).map_err(zbus::Error::from)?)
            .await?;
        if !present {
            lock(&self.inhibitors).active.remove(&cookie);
        }
        Ok(cookie)
    }

    fn un_inhibit(&self, cookie: u32, #[zbus(header)] header: Header<'_>) -> zbus::fdo::Result<()> {
        let caller = sender(&header)?;
        let mut inhibitors = lock(&self.inhibitors);
        if inhibitors
            .active
            .get(&cookie)
            .is_some_and(|inhibitor| inhibitor.owner == caller)
        {
            inhibitors.active.remove(&cookie);
        }
        Ok(())
    }
}
