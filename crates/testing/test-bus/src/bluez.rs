use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use zbus::blocking::Connection;
use zbus::names::OwnedUniqueName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedObjectPath;

use crate::SessionBus;

pub const ADAPTER: &str = "/org/bluez/hci0";
pub const HEADPHONES: &str = "/org/bluez/hci0/dev_00_1B_66_01_02_03";
pub const MOUSE: &str = "/org/bluez/hci0/dev_F0_99_B6_04_05_06";
pub const PHONE: &str = "/org/bluez/hci0/dev_5C_F9_38_07_08_09";
pub const KEYBOARD: &str = "/org/bluez/hci0/dev_E8_06_88_0A_0B_0C";
pub const PHONE_PASSKEY: u32 = 123_456;
pub const KEYBOARD_PASSKEY: u32 = 4_207;
pub const HEADPHONES_BATTERY: u8 = 80;

type AgentLocation = Arc<Mutex<Option<(OwnedUniqueName, OwnedObjectPath)>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct AgentManager {
    agent: AgentLocation,
    capability: Arc<Mutex<String>>,
}

#[zbus::interface(name = "org.bluez.AgentManager1")]
impl AgentManager {
    fn register_agent(
        &self,
        agent: OwnedObjectPath,
        capability: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) {
        let sender = header
            .sender()
            .map(|name| OwnedUniqueName::from(name.to_owned()));
        *lock(&self.agent) = sender.map(|sender| (sender, agent));
        *lock(&self.capability) = capability;
    }

    fn request_default_agent(&self, _agent: OwnedObjectPath) {}
}

struct Adapter {
    powered: bool,
    discovering: bool,
}

#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    async fn start_discovery(&mut self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.discovering = true;
        let _ = self.discovering_changed(&emitter).await;
    }

    async fn stop_discovery(&mut self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.discovering = false;
        let _ = self.discovering_changed(&emitter).await;
    }

    async fn remove_device(
        &self,
        device: OwnedObjectPath,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<()> {
        let _ = server.remove::<Battery, _>(&device).await;
        server.remove::<Device, _>(&device).await?;
        Ok(())
    }

    #[zbus(property)]
    fn powered(&self) -> bool {
        self.powered
    }

    #[zbus(property)]
    fn set_powered(&mut self, powered: bool) {
        self.powered = powered;
    }

    #[zbus(property)]
    fn discovering(&self) -> bool {
        self.discovering
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum BluezError {
    #[zbus(error)]
    ZBus(zbus::Error),
    AuthenticationRejected(String),
}

struct Device {
    address: &'static str,
    alias: &'static str,
    icon: &'static str,
    paired: AtomicBool,
    trusted: bool,
    connected: bool,
    path: &'static str,
    agent: AgentLocation,
}

#[zbus::interface(name = "org.bluez.Device1")]
impl Device {
    async fn connect(&mut self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.connected = true;
        let _ = self.connected_changed(&emitter).await;
    }

    async fn disconnect(&mut self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        self.connected = false;
        let _ = self.connected_changed(&emitter).await;
    }

    async fn pair(
        &self,
        #[zbus(connection)] bus: &zbus::Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), BluezError> {
        let (sender, agent) = lock(&self.agent)
            .clone()
            .ok_or_else(|| BluezError::AuthenticationRejected("no agent".into()))?;
        let agent = zbus::Proxy::new(bus, sender, agent, "org.bluez.Agent1").await?;
        let device = OwnedObjectPath::try_from(self.path).map_err(zbus::Error::from)?;
        let reply = if self.icon == "input-keyboard" {
            agent
                .call_method("DisplayPasskey", &(device, KEYBOARD_PASSKEY, 0u16))
                .await
        } else {
            agent
                .call_method("RequestConfirmation", &(device, PHONE_PASSKEY))
                .await
        };
        reply.map_err(|_| BluezError::AuthenticationRejected("declined".into()))?;
        self.paired.store(true, Ordering::SeqCst);
        let _ = self.paired_changed(&emitter).await;
        Ok(())
    }

    #[zbus(property)]
    fn address(&self) -> &str {
        self.address
    }

    #[zbus(property)]
    fn alias(&self) -> &str {
        self.alias
    }

    #[zbus(property)]
    fn icon(&self) -> &str {
        self.icon
    }

    #[zbus(property)]
    fn paired(&self) -> bool {
        self.paired.load(Ordering::SeqCst)
    }

    #[zbus(property)]
    fn trusted(&self) -> bool {
        self.trusted
    }

    #[zbus(property)]
    fn set_trusted(&mut self, trusted: bool) {
        self.trusted = trusted;
    }

    #[zbus(property)]
    fn connected(&self) -> bool {
        self.connected
    }

    #[zbus(property)]
    fn adapter(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from(ADAPTER).unwrap_or_default()
    }
}

struct Battery {
    percentage: u8,
}

#[zbus::interface(name = "org.bluez.Battery1")]
impl Battery {
    #[zbus(property)]
    fn percentage(&self) -> u8 {
        self.percentage
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceState {
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
}

pub struct FakeBluez {
    bluez: Connection,
    capability: Arc<Mutex<String>>,
}

impl FakeBluez {
    pub fn start(bus: &SessionBus) -> zbus::Result<Self> {
        let bluez = bus.connect()?;
        let agent = AgentLocation::default();
        let capability = Arc::default();
        let server = bluez.object_server().clone();
        server.at("/", zbus::fdo::ObjectManager)?;
        server.at(
            "/org/bluez",
            AgentManager {
                agent: Arc::clone(&agent),
                capability: Arc::clone(&capability),
            },
        )?;
        server.at(
            ADAPTER,
            Adapter {
                powered: true,
                discovering: false,
            },
        )?;
        let devices = [
            (
                HEADPHONES,
                "00:1B:66:01:02:03",
                "Studio Headphones",
                "audio-headphones",
                true,
                true,
            ),
            (
                MOUSE,
                "F0:99:B6:04:05:06",
                "mouse",
                "input-mouse",
                true,
                false,
            ),
            (PHONE, "5C:F9:38:07:08:09", "Phone", "phone", false, false),
            (
                KEYBOARD,
                "E8:06:88:0A:0B:0C",
                "Keyboard",
                "input-keyboard",
                false,
                false,
            ),
        ];
        for (path, address, alias, icon, paired, connected) in devices {
            server.at(
                path,
                Device {
                    address,
                    alias,
                    icon,
                    paired: AtomicBool::new(paired),
                    trusted: paired,
                    connected,
                    path,
                    agent: Arc::clone(&agent),
                },
            )?;
        }
        server.at(
            HEADPHONES,
            Battery {
                percentage: HEADPHONES_BATTERY,
            },
        )?;
        bluez.request_name("org.bluez")?;
        Ok(Self { bluez, capability })
    }

    pub fn capability(&self) -> String {
        lock(&self.capability).clone()
    }

    fn adapter(&self) -> zbus::Result<zbus::blocking::object_server::InterfaceRef<Adapter>> {
        self.bluez.object_server().interface::<_, Adapter>(ADAPTER)
    }

    pub fn powered(&self) -> zbus::Result<bool> {
        Ok(self.adapter()?.get().powered)
    }

    pub fn discovering(&self) -> zbus::Result<bool> {
        Ok(self.adapter()?.get().discovering)
    }

    pub fn knows(&self, path: &str) -> bool {
        self.bluez
            .object_server()
            .interface::<_, Device>(path)
            .is_ok()
    }

    pub fn device(&self, path: &str) -> zbus::Result<DeviceState> {
        let device = self.bluez.object_server().interface::<_, Device>(path)?;
        let device = device.get();
        Ok(DeviceState {
            paired: device.paired.load(Ordering::SeqCst),
            trusted: device.trusted,
            connected: device.connected,
        })
    }
}
