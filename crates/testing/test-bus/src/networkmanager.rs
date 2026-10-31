use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

use zbus::blocking::Connection;
use zbus::fdo::Error::UnknownObject;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::SessionBus;

pub type Settings = HashMap<String, HashMap<String, OwnedValue>>;

pub const ROOT: &str = "/org/freedesktop/NetworkManager";
pub const WIFI: &str = "/org/freedesktop/NetworkManager/Devices/1";
pub const ETHERNET: &str = "/org/freedesktop/NetworkManager/Devices/2";
pub const SAVED: &str = "/org/freedesktop/NetworkManager/Settings/100";
const RSN_PSK: u32 = 0x100 | 0x8 | 0x80;
const RSN_SAE: u32 = 0x400 | 0x8 | 0x80;
const RSN_EAP: u32 = 0x200 | 0x8 | 0x80;
const PRIVACY: u32 = 0x1;
const ACTIVATION_DELAY: Duration = Duration::from_millis(100);
const CONNECTIVITY_FULL: u32 = 4;
pub const CONNECTIVITY_PORTAL: u32 = 2;
pub const CHECK_URI: &str = "http://check.example/";

#[derive(Default)]
struct World {
    passwords: HashMap<Vec<u8>, String>,
    connections: Vec<(OwnedObjectPath, Settings)>,
    next: u32,
}

type Shared = Arc<Mutex<World>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn path(text: String) -> zbus::fdo::Result<OwnedObjectPath> {
    OwnedObjectPath::try_from(text)
        .map_err(|error| zbus::fdo::Error::InvalidArgs(error.to_string()))
}

pub fn ssid_of(settings: &Settings) -> Option<Vec<u8>> {
    settings
        .get("802-11-wireless")?
        .get("ssid")?
        .try_clone()
        .ok()?
        .try_into()
        .ok()
}

fn psk_of(settings: &Settings) -> Option<String> {
    let psk = settings.get("802-11-wireless-security")?.get("psk")?;
    String::try_from(psk.try_clone().ok()?).ok()
}

struct Manager {
    world: Shared,
    wireless_enabled: bool,
    connectivity: u32,
}

impl Manager {
    async fn activate(
        &self,
        server: &zbus::ObjectServer,
        connection: &zbus::Connection,
        settings: &Settings,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let succeeds = {
            let world = lock(&self.world);
            ssid_of(settings).and_then(|ssid| world.passwords.get(&ssid).cloned())
                == psk_of(settings)
        };
        let active = {
            let mut world = lock(&self.world);
            world.next += 1;
            path(format!("{ROOT}/ActiveConnection/{}", world.next))?
        };
        server.at(&active, Active { state: 1 }).await?;
        let connection = connection.clone();
        let emitted = active.clone();
        thread::spawn(move || {
            thread::sleep(ACTIVATION_DELAY);
            zbus::block_on(async {
                let (state, reason) = if succeeds { (2, 1) } else { (4, 9) };
                let Ok(interface) = connection
                    .object_server()
                    .interface::<_, Active>(&emitted)
                    .await
                else {
                    return;
                };
                interface.get_mut().await.state = state;
                let _ = Active::activation_state_changed(interface.signal_emitter(), state, reason)
                    .await;
            });
        });
        Ok(active)
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    fn get_devices(&self) -> zbus::fdo::Result<Vec<OwnedObjectPath>> {
        Ok(vec![path(ETHERNET.into())?, path(WIFI.into())?])
    }

    async fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        _device: OwnedObjectPath,
        _specific_object: OwnedObjectPath,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] bus: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let settings = lock(&self.world)
            .connections
            .iter()
            .find(|(saved, _)| *saved == connection)
            .map(|(_, settings)| settings.clone())
            .ok_or_else(|| UnknownObject(connection.to_string()))?;
        self.activate(server, bus, &settings).await
    }

    async fn add_and_activate_connection(
        &self,
        settings: Settings,
        _device: OwnedObjectPath,
        _specific_object: OwnedObjectPath,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] bus: &zbus::Connection,
    ) -> zbus::fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
        let saved = {
            let mut world = lock(&self.world);
            world.next += 1;
            let saved = path(format!("{ROOT}/Settings/{}", world.next))?;
            world.connections.push((saved.clone(), settings.clone()));
            saved
        };
        server
            .at(
                &saved,
                SavedConnection {
                    world: self.world.clone(),
                    path: saved.clone(),
                },
            )
            .await?;
        let active = self.activate(server, bus, &settings).await?;
        Ok((saved, active))
    }

    #[zbus(property)]
    fn wireless_enabled(&self) -> bool {
        self.wireless_enabled
    }

    #[zbus(property)]
    fn set_wireless_enabled(&mut self, enabled: bool) {
        self.wireless_enabled = enabled;
    }

    #[zbus(property)]
    fn connectivity(&self) -> u32 {
        self.connectivity
    }

    #[zbus(property)]
    fn connectivity_check_uri(&self) -> &str {
        CHECK_URI
    }
}

struct Device {
    device_type: u32,
    state: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn device_type(&self) -> u32 {
        self.device_type
    }

    #[zbus(property)]
    fn state(&self) -> u32 {
        self.state
    }
}

struct WiredDevice;

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wired")]
impl WiredDevice {
    #[zbus(property)]
    fn carrier(&self) -> bool {
        true
    }
}

struct Wireless {
    points: Vec<OwnedObjectPath>,
    active: OwnedObjectPath,
    scans: Arc<Mutex<u32>>,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    fn get_all_access_points(&self) -> Vec<OwnedObjectPath> {
        self.points.clone()
    }

    fn request_scan(&self, _options: HashMap<String, OwnedValue>) {
        *lock(&self.scans) += 1;
    }

    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        self.active.clone()
    }
}

struct AccessPoint {
    ssid: &'static [u8],
    strength: u8,
    flags: u32,
    rsn_flags: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn ssid(&self) -> Vec<u8> {
        self.ssid.to_vec()
    }

    #[zbus(property)]
    fn strength(&self) -> u8 {
        self.strength
    }

    #[zbus(property)]
    fn flags(&self) -> u32 {
        self.flags
    }

    #[zbus(property)]
    fn wpa_flags(&self) -> u32 {
        0
    }

    #[zbus(property)]
    fn rsn_flags(&self) -> u32 {
        self.rsn_flags
    }
}

struct SettingsRoot {
    world: Shared,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl SettingsRoot {
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        lock(&self.world)
            .connections
            .iter()
            .map(|(path, _)| path.clone())
            .collect()
    }
}

struct SavedConnection {
    world: Shared,
    path: OwnedObjectPath,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl SavedConnection {
    fn get_settings(&self) -> zbus::fdo::Result<Settings> {
        let world = lock(&self.world);
        let (_, settings) = world
            .connections
            .iter()
            .find(|(path, _)| *path == self.path)
            .ok_or_else(|| UnknownObject(self.path.to_string()))?;
        let mut settings = settings.clone();
        if let Some(security) = settings.get_mut("802-11-wireless-security") {
            security.remove("psk");
        }
        Ok(settings)
    }

    fn update(&self, properties: Settings) {
        if let Some((_, settings)) = lock(&self.world)
            .connections
            .iter_mut()
            .find(|(path, _)| *path == self.path)
        {
            *settings = properties;
        }
    }

    async fn delete(
        &self,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        lock(&self.world)
            .connections
            .retain(|(path, _)| *path != self.path);
        server.remove::<Self, _>(&self.path).await?;
        Ok(())
    }
}

struct Active {
    state: u32,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.state
    }

    #[zbus(signal, name = "StateChanged")]
    async fn activation_state_changed(
        emitter: &SignalEmitter<'_>,
        state: u32,
        reason: u32,
    ) -> zbus::Result<()>;
}

fn owned(value: Value<'_>) -> zbus::Result<OwnedValue> {
    Ok(OwnedValue::try_from(value)?)
}

pub fn wireless_settings(ssid: &[u8], psk: Option<&str>) -> zbus::Result<Settings> {
    let mut settings = HashMap::from([(
        "802-11-wireless".to_owned(),
        HashMap::from([("ssid".to_owned(), owned(Value::from(ssid.to_vec()))?)]),
    )]);
    if let Some(psk) = psk {
        settings.insert(
            "802-11-wireless-security".to_owned(),
            HashMap::from([
                ("key-mgmt".to_owned(), owned(Value::from("wpa-psk"))?),
                ("psk".to_owned(), owned(Value::from(psk))?),
            ]),
        );
    }
    Ok(settings)
}

pub struct FakeNetworkManager {
    daemon: Connection,
    world: Shared,
    scans: Arc<Mutex<u32>>,
}

impl FakeNetworkManager {
    pub const HOME_PASSWORD: &str = "correct horse";
    pub const STUDIO_PASSWORD: &str = "battery staple";

    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let daemon = bus.connect()?;
        let world: Shared = Arc::default();
        {
            let mut world = lock(&world);
            world
                .passwords
                .insert(b"Home".to_vec(), Self::HOME_PASSWORD.into());
            world
                .passwords
                .insert(b"Studio".to_vec(), Self::STUDIO_PASSWORD.into());
            world.connections.push((
                path(SAVED.into())?,
                wireless_settings(b"Studio", Some("old password"))?,
            ));
        }
        let points: [(&[u8], u8, u32, u32); 7] = [
            (b"Home", 40, PRIVACY, RSN_PSK),
            (b"Home", 70, PRIVACY, RSN_PSK),
            (b"Cafe", 90, 0, 0),
            (b"Studio", 30, PRIVACY, RSN_PSK),
            (b"Office", 80, PRIVACY, RSN_EAP),
            (b"Modern", 60, PRIVACY, RSN_SAE),
            (b"", 99, PRIVACY, RSN_PSK),
        ];
        let server = daemon.object_server().clone();
        let mut paths = Vec::new();
        for (index, (ssid, strength, flags, rsn_flags)) in points.into_iter().enumerate() {
            let point = path(format!("{ROOT}/AccessPoint/{index}"))?;
            server.at(
                &point,
                AccessPoint {
                    ssid,
                    strength,
                    flags,
                    rsn_flags,
                },
            )?;
            paths.push(point);
        }
        let scans = Arc::default();
        server.at(
            ROOT,
            Manager {
                world: world.clone(),
                wireless_enabled: true,
                connectivity: CONNECTIVITY_FULL,
            },
        )?;
        server.at(
            WIFI,
            Device {
                device_type: 2,
                state: 100,
            },
        )?;
        server.at(
            WIFI,
            Wireless {
                active: paths[0].clone(),
                points: paths,
                scans: Arc::clone(&scans),
            },
        )?;
        server.at(
            ETHERNET,
            Device {
                device_type: 1,
                state: 30,
            },
        )?;
        server.at(ETHERNET, WiredDevice)?;
        server.at(
            format!("{ROOT}/Settings"),
            SettingsRoot {
                world: world.clone(),
            },
        )?;
        server.at(
            SAVED,
            SavedConnection {
                world: world.clone(),
                path: path(SAVED.into())?,
            },
        )?;
        daemon.request_name("org.freedesktop.NetworkManager")?;
        Ok(Self {
            daemon,
            world,
            scans,
        })
    }

    pub fn wireless_enabled(&self) -> zbus::Result<bool> {
        Ok(self
            .daemon
            .object_server()
            .interface::<_, Manager>(ROOT)?
            .get()
            .wireless_enabled)
    }

    pub fn set_connectivity(&self, connectivity: u32) -> zbus::Result<()> {
        let manager = self.daemon.object_server().interface::<_, Manager>(ROOT)?;
        manager.get_mut().connectivity = connectivity;
        zbus::block_on(manager.get().connectivity_changed(manager.signal_emitter()))
    }

    pub fn scans(&self) -> u32 {
        *lock(&self.scans)
    }

    pub fn saved_ssids(&self) -> Vec<Vec<u8>> {
        lock(&self.world)
            .connections
            .iter()
            .filter_map(|(_, settings)| ssid_of(settings))
            .collect()
    }
}
