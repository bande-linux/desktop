use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use unbound_bluetooth::{Bluetooth, PairingPrompt, register_agent};
use unbound_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::names::OwnedUniqueName;
use zbus::zvariant::OwnedObjectPath;

const ADAPTER: &str = "/org/bluez/hci0";
const HEADPHONES: &str = "/org/bluez/hci0/dev_00_1B_66_01_02_03";
const MOUSE: &str = "/org/bluez/hci0/dev_F0_99_B6_04_05_06";
const PHONE: &str = "/org/bluez/hci0/dev_5C_F9_38_07_08_09";
const KEYBOARD: &str = "/org/bluez/hci0/dev_E8_06_88_0A_0B_0C";
const PHONE_PASSKEY: u32 = 123_456;
const KEYBOARD_PASSKEY: u32 = 4_207;

type AgentLocation = Arc<Mutex<Option<(OwnedUniqueName, OwnedObjectPath)>>>;

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
        *self.agent.lock().expect("agent") = sender.map(|sender| (sender, agent));
        *self.capability.lock().expect("capability") = capability;
    }

    fn request_default_agent(&self, _agent: OwnedObjectPath) {}
}

struct Adapter {
    powered: bool,
    discovering: bool,
}

#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    fn start_discovery(&mut self) {
        self.discovering = true;
    }

    fn stop_discovery(&mut self) {
        self.discovering = false;
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
    fn connect(&mut self) {
        self.connected = true;
    }

    fn disconnect(&mut self) {
        self.connected = false;
    }

    async fn pair(&self, #[zbus(connection)] bus: &zbus::Connection) -> Result<(), BluezError> {
        let (sender, agent) = self
            .agent
            .lock()
            .expect("agent")
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
}

#[derive(Default)]
struct Prompt {
    accept: bool,
    asked: Mutex<Vec<String>>,
}

impl PairingPrompt for Prompt {
    fn confirm_passkey(&self, device: &str, passkey: u32) -> bool {
        self.asked
            .lock()
            .expect("asked")
            .push(format!("confirm {device} {passkey:06}"));
        self.accept
    }

    fn show_code(&self, device: &str, code: &str) {
        self.asked
            .lock()
            .expect("asked")
            .push(format!("show {device} {code}"));
    }

    fn ask_pin_code(&self, _device: &str) -> Option<String> {
        None
    }

    fn authorize(&self, _device: &str) -> bool {
        self.accept
    }

    fn dismiss(&self) {}
}

struct Fixture {
    bluez: Connection,
    capability: Arc<Mutex<String>>,
    prompt: Arc<Prompt>,
    client: Bluetooth,
    _bus: SessionBus,
}

fn start(accept: bool) -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let bluez = bus.connect().expect("bluez connection");
    let agent = AgentLocation::default();
    let capability = Arc::default();
    {
        let server = bluez.object_server();
        server
            .at("/", zbus::fdo::ObjectManager)
            .expect("object manager");
        server
            .at(
                "/org/bluez",
                AgentManager {
                    agent: Arc::clone(&agent),
                    capability: Arc::clone(&capability),
                },
            )
            .expect("agent manager");
        server
            .at(
                ADAPTER,
                Adapter {
                    powered: true,
                    discovering: false,
                },
            )
            .expect("adapter");
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
            server
                .at(
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
                )
                .expect("device");
        }
    }
    bluez.request_name("org.bluez").expect("name");
    let client_bus = bus.connect().expect("client connection");
    let prompt = Arc::new(Prompt {
        accept,
        ..Prompt::default()
    });
    register_agent(&client_bus, prompt.clone()).expect("agent");
    Fixture {
        bluez,
        capability,
        prompt,
        client: Bluetooth::new(&client_bus),
        _bus: bus,
    }
}

impl Fixture {
    fn device(&self, path: &str) -> zbus::blocking::object_server::InterfaceRef<Device> {
        self.bluez
            .object_server()
            .interface::<_, Device>(path)
            .expect("device")
    }

    fn adapter(&self) -> zbus::blocking::object_server::InterfaceRef<Adapter> {
        self.bluez
            .object_server()
            .interface::<_, Adapter>(ADAPTER)
            .expect("adapter")
    }

    fn named(&self, name: &str) -> unbound_bluetooth::Device {
        self.client
            .paired_devices()
            .expect("paired")
            .into_iter()
            .chain(self.client.nearby_devices().expect("nearby"))
            .find(|device| device.name == name)
            .expect("device listed")
    }

    fn asked(&self) -> Vec<String> {
        self.prompt.asked.lock().expect("asked").clone()
    }
}

#[test]
fn turning_bluetooth_off_and_on_writes_the_adapter_power() {
    let fixture = start(true);
    assert!(fixture.client.available().expect("available"));
    assert!(fixture.client.powered().expect("powered"));
    fixture.client.set_powered(false).expect("off");
    assert!(!fixture.adapter().get().powered);
    fixture.client.set_powered(true).expect("on");
    assert!(fixture.adapter().get().powered);
}

#[test]
fn paired_devices_list_connected_first_then_by_name() {
    let fixture = start(true);
    let paired: Vec<(String, bool, Option<String>)> = fixture
        .client
        .paired_devices()
        .expect("paired")
        .into_iter()
        .map(|device| (device.name, device.connected, device.icon))
        .collect();
    assert_eq!(
        paired,
        [
            (
                "Studio Headphones".to_owned(),
                true,
                Some("audio-headphones".to_owned())
            ),
            ("mouse".to_owned(), false, Some("input-mouse".to_owned())),
        ]
    );
    let nearby: Vec<String> = fixture
        .client
        .nearby_devices()
        .expect("nearby")
        .into_iter()
        .map(|device| device.address)
        .collect();
    assert_eq!(nearby, ["E8:06:88:0A:0B:0C", "5C:F9:38:07:08:09"]);
}

#[test]
fn connecting_and_disconnecting_a_paired_device() {
    let fixture = start(true);
    fixture
        .client
        .connect(&fixture.named("mouse"))
        .expect("connect");
    assert!(fixture.device(MOUSE).get().connected);
    fixture
        .client
        .disconnect(&fixture.named("Studio Headphones"))
        .expect("disconnect");
    assert!(!fixture.device(HEADPHONES).get().connected);
}

#[test]
fn discovery_runs_only_while_asked() {
    let fixture = start(true);
    fixture.client.set_discovering(true).expect("start");
    assert!(fixture.adapter().get().discovering);
    fixture.client.set_discovering(false).expect("stop");
    assert!(!fixture.adapter().get().discovering);
}

#[test]
fn pairing_confirms_the_passkey_then_trusts_and_connects() {
    let fixture = start(true);
    assert_eq!(
        *fixture.capability.lock().expect("capability"),
        "KeyboardDisplay"
    );
    fixture.client.pair(&fixture.named("Phone")).expect("pair");
    assert_eq!(fixture.asked(), ["confirm Phone 123456"]);
    let phone = fixture.device(PHONE);
    let phone = phone.get();
    assert!(phone.paired.load(Ordering::SeqCst) && phone.trusted && phone.connected);
    assert!(
        fixture
            .client
            .paired_devices()
            .expect("paired")
            .iter()
            .any(|device| device.name == "Phone")
    );
}

#[test]
fn a_keyboard_shows_its_passkey_as_six_digits() {
    let fixture = start(true);
    fixture
        .client
        .pair(&fixture.named("Keyboard"))
        .expect("pair");
    assert_eq!(fixture.asked(), ["show Keyboard 004207"]);
}

#[test]
fn declining_the_passkey_leaves_the_device_unpaired() {
    let fixture = start(false);
    assert!(fixture.client.pair(&fixture.named("Phone")).is_err());
    let phone = fixture.device(PHONE);
    let phone = phone.get();
    assert!(!phone.paired.load(Ordering::SeqCst) && !phone.trusted && !phone.connected);
}
