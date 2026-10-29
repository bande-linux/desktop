#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use unbound_network::{Error, JoinOutcome, NetworkManager, Security, Wired};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

type Settings = HashMap<String, HashMap<String, OwnedValue>>;

const ROOT: &str = "/org/freedesktop/NetworkManager";
const WIFI: &str = "/org/freedesktop/NetworkManager/Devices/1";
const ETHERNET: &str = "/org/freedesktop/NetworkManager/Devices/2";
const RSN_PSK: u32 = 0x100 | 0x8 | 0x80;
const RSN_SAE: u32 = 0x400 | 0x8 | 0x80;
const RSN_EAP: u32 = 0x200 | 0x8 | 0x80;
const PRIVACY: u32 = 0x1;

#[derive(Default)]
struct World {
    passwords: HashMap<Vec<u8>, String>,
    connections: Vec<(OwnedObjectPath, Settings)>,
    next: u32,
}

type Shared = Arc<Mutex<World>>;

fn path(text: String) -> OwnedObjectPath {
    OwnedObjectPath::try_from(text).expect("object path")
}

fn ssid_of(settings: &Settings) -> Vec<u8> {
    settings["802-11-wireless"]["ssid"]
        .clone()
        .try_into()
        .expect("ssid bytes")
}

fn psk_of(settings: &Settings) -> Option<String> {
    let psk = settings.get("802-11-wireless-security")?.get("psk")?;
    String::try_from(psk.clone()).ok()
}

struct Manager {
    world: Shared,
    wireless_enabled: bool,
}

impl Manager {
    async fn activate(
        &self,
        server: &zbus::ObjectServer,
        connection: &zbus::Connection,
        settings: &Settings,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let ssid = ssid_of(settings);
        let succeeds = {
            let world = self.world.lock().expect("world");
            world.passwords.get(&ssid).cloned() == psk_of(settings)
        };
        let active = {
            let mut world = self.world.lock().expect("world");
            world.next += 1;
            path(format!("{ROOT}/ActiveConnection/{}", world.next))
        };
        server.at(&active, Active { state: 1 }).await?;
        let connection = connection.clone();
        let emitted = active.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            zbus::block_on(async {
                let (state, reason) = if succeeds { (2, 1) } else { (4, 9) };
                let interface = connection
                    .object_server()
                    .interface::<_, Active>(&emitted)
                    .await
                    .expect("active connection");
                interface.get_mut().await.state = state;
                Active::activation_state_changed(interface.signal_emitter(), state, reason)
                    .await
                    .expect("signal");
            });
        });
        Ok(active)
    }
}

#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    fn get_devices(&self) -> Vec<OwnedObjectPath> {
        vec![path(ETHERNET.into()), path(WIFI.into())]
    }

    async fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        _device: OwnedObjectPath,
        _specific_object: OwnedObjectPath,
        #[zbus(object_server)] server: &zbus::ObjectServer,
        #[zbus(connection)] bus: &zbus::Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let settings = self
            .world
            .lock()
            .expect("world")
            .connections
            .iter()
            .find(|(saved, _)| *saved == connection)
            .map(|(_, settings)| settings.clone())
            .ok_or_else(|| zbus::fdo::Error::UnknownObject(connection.to_string()))?;
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
            let mut world = self.world.lock().expect("world");
            world.next += 1;
            let saved = path(format!("{ROOT}/Settings/{}", world.next));
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
        *self.scans.lock().expect("scans") += 1;
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
        let world = self.world.lock().expect("world");
        world
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
    fn get_settings(&self) -> Settings {
        let world = self.world.lock().expect("world");
        let (_, settings) = world
            .connections
            .iter()
            .find(|(path, _)| *path == self.path)
            .expect("saved connection");
        let mut settings = settings.clone();
        if let Some(security) = settings.get_mut("802-11-wireless-security") {
            security.remove("psk");
        }
        settings
    }

    fn update(&self, properties: Settings) {
        let mut world = self.world.lock().expect("world");
        if let Some((_, settings)) = world
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
        self.world
            .lock()
            .expect("world")
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

struct Fixture {
    daemon: Connection,
    world: Shared,
    scans: Arc<Mutex<u32>>,
    client: NetworkManager,
    _bus: SessionBus,
}

fn wireless_settings(ssid: &[u8], psk: Option<&str>) -> Settings {
    let mut settings = HashMap::from([(
        "802-11-wireless".to_owned(),
        HashMap::from([(
            "ssid".to_owned(),
            OwnedValue::try_from(Value::from(ssid.to_vec())).expect("ssid"),
        )]),
    )]);
    if let Some(psk) = psk {
        settings.insert(
            "802-11-wireless-security".to_owned(),
            HashMap::from([
                (
                    "key-mgmt".to_owned(),
                    OwnedValue::try_from(Value::from("wpa-psk")).expect("key-mgmt"),
                ),
                (
                    "psk".to_owned(),
                    OwnedValue::try_from(Value::from(psk)).expect("psk"),
                ),
            ]),
        );
    }
    settings
}

fn start() -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let daemon = bus.connect().expect("daemon connection");
    let world: Shared = Arc::default();
    {
        let mut world = world.lock().expect("world");
        world
            .passwords
            .insert(b"Home".to_vec(), "correct horse".into());
        world
            .passwords
            .insert(b"Studio".to_vec(), "battery staple".into());
        world.connections.push((
            path(format!("{ROOT}/Settings/100")),
            wireless_settings(b"Studio", Some("old password")),
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
        let point = path(format!("{ROOT}/AccessPoint/{index}"));
        server
            .at(
                &point,
                AccessPoint {
                    ssid,
                    strength,
                    flags,
                    rsn_flags,
                },
            )
            .expect("access point");
        paths.push(point);
    }
    let scans = Arc::default();
    server
        .at(
            ROOT,
            Manager {
                world: world.clone(),
                wireless_enabled: true,
            },
        )
        .expect("manager");
    server
        .at(
            WIFI,
            Device {
                device_type: 2,
                state: 100,
            },
        )
        .expect("wifi device");
    server
        .at(
            WIFI,
            Wireless {
                active: paths[0].clone(),
                points: paths,
                scans: Arc::clone(&scans),
            },
        )
        .expect("wireless");
    server
        .at(
            ETHERNET,
            Device {
                device_type: 1,
                state: 30,
            },
        )
        .expect("ethernet device");
    server.at(ETHERNET, WiredDevice).expect("wired");
    server
        .at(
            format!("{ROOT}/Settings"),
            SettingsRoot {
                world: world.clone(),
            },
        )
        .expect("settings");
    server
        .at(
            format!("{ROOT}/Settings/100"),
            SavedConnection {
                world: world.clone(),
                path: path(format!("{ROOT}/Settings/100")),
            },
        )
        .expect("saved studio");
    daemon
        .request_name("org.freedesktop.NetworkManager")
        .expect("name");
    let client = NetworkManager::new(&bus.connect().expect("client connection"));
    Fixture {
        daemon,
        world,
        scans,
        client,
        _bus: bus,
    }
}

impl Fixture {
    fn network(&self, name: &str) -> unbound_network::Network {
        self.client
            .networks()
            .expect("networks")
            .into_iter()
            .find(|network| network.name() == name)
            .expect("network listed")
    }

    fn saved_ssids(&self) -> Vec<Vec<u8>> {
        let world = self.world.lock().expect("world");
        world
            .connections
            .iter()
            .map(|(_, settings)| ssid_of(settings))
            .collect()
    }
}

#[test]
fn turning_wifi_off_and_on_writes_wireless_enabled() {
    let fixture = start();
    assert!(fixture.client.wifi_enabled().expect("read"));
    fixture.client.set_wifi_enabled(false).expect("off");
    let manager = fixture
        .daemon
        .object_server()
        .interface::<_, Manager>(ROOT)
        .expect("manager");
    assert!(!manager.get().wireless_enabled);
    fixture.client.set_wifi_enabled(true).expect("on");
    assert!(manager.get().wireless_enabled);
}

#[test]
fn network_list_shows_one_row_per_name_current_then_known_then_strongest() {
    let fixture = start();
    let networks = fixture.client.networks().expect("networks");
    let rows: Vec<(String, u8, bool, bool)> = networks
        .iter()
        .map(|network| {
            (
                network.name(),
                network.strength,
                network.current,
                network.known,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Home".to_owned(), 70, true, false),
            ("Studio".to_owned(), 30, false, true),
            ("Cafe".to_owned(), 90, false, false),
            ("Office".to_owned(), 80, false, false),
            ("Modern".to_owned(), 60, false, false),
        ]
    );
    let security: Vec<Security> = networks.iter().map(|network| network.security).collect();
    assert_eq!(
        security,
        [
            Security::Personal { sae_only: false },
            Security::Personal { sae_only: false },
            Security::Open,
            Security::Enterprise,
            Security::Personal { sae_only: true },
        ]
    );
}

#[test]
fn opening_the_list_asks_for_a_scan() {
    let fixture = start();
    fixture.client.scan().expect("scan");
    assert_eq!(*fixture.scans.lock().expect("scans"), 1);
}

#[test]
fn joining_an_open_network_needs_no_password_and_remembers_it() {
    let fixture = start();
    let cafe = fixture.network("Cafe");
    assert_eq!(
        fixture.client.join(&cafe, None).expect("join"),
        JoinOutcome::Joined
    );
    assert!(fixture.saved_ssids().contains(&b"Cafe".to_vec()));
    assert!(fixture.network("Cafe").known);
}

#[test]
fn a_secured_unknown_network_asks_for_a_password_first() {
    let fixture = start();
    let home = fixture.network("Home");
    assert!(matches!(
        fixture.client.join(&home, None),
        Err(Error::PasswordRequired)
    ));
    assert_eq!(fixture.saved_ssids(), [b"Studio".to_vec()]);
}

#[test]
fn a_wrong_password_is_reported_and_not_remembered() {
    let fixture = start();
    let home = fixture.network("Home");
    assert_eq!(
        fixture.client.join(&home, Some("wrong")).expect("join"),
        JoinOutcome::WrongPassword
    );
    assert_eq!(fixture.saved_ssids(), [b"Studio".to_vec()]);
    assert_eq!(
        fixture
            .client
            .join(&home, Some("correct horse"))
            .expect("join"),
        JoinOutcome::Joined
    );
    assert!(fixture.saved_ssids().contains(&b"Home".to_vec()));
}

#[test]
fn a_known_network_rejoins_with_its_saved_password_or_a_new_one() {
    let fixture = start();
    let studio = fixture.network("Studio");
    assert_eq!(
        fixture.client.join(&studio, None).expect("join"),
        JoinOutcome::WrongPassword
    );
    assert_eq!(
        fixture
            .client
            .join(&studio, Some("battery staple"))
            .expect("join"),
        JoinOutcome::Joined
    );
    assert_eq!(fixture.saved_ssids(), [b"Studio".to_vec()]);
}

#[test]
fn enterprise_networks_are_not_joined_with_a_personal_password() {
    let fixture = start();
    let office = fixture.network("Office");
    assert!(matches!(
        fixture.client.join(&office, Some("secret")),
        Err(Error::Unsupported(Security::Enterprise))
    ));
}

#[test]
fn wired_status_reports_cable_and_connection() {
    let fixture = start();
    assert_eq!(
        fixture.client.wired().expect("wired"),
        Some(Wired {
            cable: true,
            connected: false
        })
    );
}
