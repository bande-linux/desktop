mod agent;

use std::collections::HashMap;

pub use agent::{AGENT_PATH, PairingPrompt, register_agent};
use zbus::blocking::Connection;
use zbus::blocking::fdo::{DBusProxy, ObjectManagerProxy};
use zbus::names::OwnedInterfaceName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const SERVICE: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";

#[zbus::proxy(interface = "org.bluez.Adapter1", default_service = "org.bluez")]
trait Adapter {
    fn start_discovery(&self) -> zbus::Result<()>;

    fn stop_discovery(&self) -> zbus::Result<()>;

    fn remove_device(&self, device: &zbus::zvariant::ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_powered(&self, powered: bool) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.bluez.Device1", default_service = "org.bluez")]
trait Device {
    fn connect(&self) -> zbus::Result<()>;

    fn disconnect(&self) -> zbus::Result<()>;

    fn pair(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn set_trusted(&self, trusted: bool) -> zbus::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Mouse,
    Keyboard,
    Tablet,
    GameController,
    Headphones,
    Headset,
    Speaker,
    Phone,
    Computer,
    Camera,
    Printer,
    Display,
    Other,
}

impl Kind {
    fn from_icon(icon: &str) -> Self {
        match icon {
            "input-mouse" => Self::Mouse,
            "input-keyboard" => Self::Keyboard,
            "input-tablet" => Self::Tablet,
            "input-gaming" => Self::GameController,
            "audio-headphones" => Self::Headphones,
            "audio-headset" => Self::Headset,
            "audio-card" => Self::Speaker,
            "phone" => Self::Phone,
            "computer" => Self::Computer,
            "camera-photo" | "camera-video" => Self::Camera,
            "printer" => Self::Printer,
            "video-display" => Self::Display,
            _ => Self::Other,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Mouse => "input-mouse-symbolic",
            Self::Keyboard => "input-keyboard-symbolic",
            Self::Tablet => "input-tablet-symbolic",
            Self::GameController => "input-gaming-symbolic",
            Self::Headphones => "audio-headphones-symbolic",
            Self::Headset => "audio-headset-symbolic",
            Self::Speaker => "audio-card-symbolic",
            Self::Phone => "phone-symbolic",
            Self::Computer => "computer-symbolic",
            Self::Camera => "camera-photo-symbolic",
            Self::Printer => "printer-symbolic",
            Self::Display => "video-display-symbolic",
            Self::Other => "bluetooth-symbolic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub address: String,
    pub name: String,
    pub named: bool,
    pub kind: Kind,
    pub paired: bool,
    pub connected: bool,
    pub battery: Option<u8>,
    path: OwnedObjectPath,
}

type Interfaces = HashMap<OwnedInterfaceName, HashMap<String, OwnedValue>>;

fn property<T: TryFrom<OwnedValue>>(
    properties: &HashMap<String, OwnedValue>,
    name: &str,
) -> Option<T> {
    properties
        .get(name)
        .and_then(|value| T::try_from(value.clone()).ok())
}

impl Device {
    fn from_interfaces(path: OwnedObjectPath, interfaces: &Interfaces) -> Option<Self> {
        let of = |interface: &str| {
            interfaces
                .iter()
                .find(|(name, _)| name.as_str() == interface)
                .map(|(_, properties)| properties)
        };
        let properties = of(DEVICE)?;
        let address: String = property(properties, "Address")?;
        let alias = property::<String>(properties, "Alias")
            .filter(|alias| *alias != address.replace(':', "-"));
        let named = alias.is_some() || properties.contains_key("Name");
        let icon: Option<String> = property(properties, "Icon");
        Some(Self {
            name: alias.unwrap_or_else(|| address.clone()),
            named,
            kind: icon.as_deref().map_or(Kind::Other, Kind::from_icon),
            paired: property(properties, "Paired").unwrap_or(false),
            connected: property(properties, "Connected").unwrap_or(false),
            battery: of(BATTERY).and_then(|battery| property(battery, "Percentage")),
            address,
            path,
        })
    }
}

const SHOWN: [(&str, &[&str]); 3] = [
    (ADAPTER, &["Powered"]),
    (
        DEVICE,
        &["Address", "Alias", "Name", "Icon", "Paired", "Connected"],
    ),
    (BATTERY, &["Percentage"]),
];

pub fn shown_change(message: &zbus::Message) -> bool {
    if message
        .header()
        .member()
        .is_none_or(|member| member.as_str() != "PropertiesChanged")
    {
        return true;
    }
    let Ok((interface, changed, invalidated)) =
        message
            .body()
            .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
    else {
        return true;
    };
    SHOWN
        .iter()
        .find(|(shown, _)| *shown == interface)
        .is_some_and(|(_, names)| {
            changed
                .keys()
                .chain(&invalidated)
                .any(|name| names.contains(&name.as_str()))
        })
}

fn bluez_running(bus: &Connection) -> zbus::Result<bool> {
    Ok(DBusProxy::new(bus)?.name_has_owner(SERVICE.try_into()?)?)
}

pub struct Bluetooth {
    bus: Connection,
}

impl Bluetooth {
    pub fn new(system: &Connection) -> Self {
        Self {
            bus: system.clone(),
        }
    }

    fn objects(&self) -> zbus::Result<HashMap<OwnedObjectPath, Interfaces>> {
        if !bluez_running(&self.bus)? {
            return Ok(HashMap::new());
        }
        let manager = ObjectManagerProxy::builder(&self.bus)
            .destination(SERVICE)?
            .path("/")?
            .build()?;
        Ok(manager.get_managed_objects()?)
    }

    fn adapter(&self) -> zbus::Result<Option<AdapterProxyBlocking<'static>>> {
        let mut adapters: Vec<OwnedObjectPath> = self
            .objects()?
            .into_iter()
            .filter(|(_, interfaces)| interfaces.keys().any(|name| name.as_str() == ADAPTER))
            .map(|(path, _)| path)
            .collect();
        adapters.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        adapters
            .into_iter()
            .next()
            .map(|path| AdapterProxyBlocking::builder(&self.bus).path(path)?.build())
            .transpose()
    }

    pub fn available(&self) -> zbus::Result<bool> {
        Ok(self.adapter()?.is_some())
    }

    pub fn powered(&self) -> zbus::Result<bool> {
        match self.adapter()? {
            Some(adapter) => adapter.powered(),
            None => Ok(false),
        }
    }

    pub fn set_powered(&self, powered: bool) -> zbus::Result<()> {
        match self.adapter()? {
            Some(adapter) => adapter.set_powered(powered),
            None => Err(zbus::Error::Failure(
                "this computer has no Bluetooth adapter".into(),
            )),
        }
    }

    fn devices(&self, paired: bool) -> zbus::Result<Vec<Device>> {
        let mut devices: Vec<Device> = self
            .objects()?
            .into_iter()
            .filter_map(|(path, interfaces)| Device::from_interfaces(path, &interfaces))
            .filter(|device| device.paired == paired)
            .collect();
        devices.sort_by(|a, b| {
            b.connected
                .cmp(&a.connected)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.address.cmp(&b.address))
        });
        Ok(devices)
    }

    pub fn paired_devices(&self) -> zbus::Result<Vec<Device>> {
        self.devices(true)
    }

    pub fn nearby_devices(&self) -> zbus::Result<Vec<Device>> {
        let mut devices = self.devices(false)?;
        devices.retain(|device| device.named);
        Ok(devices)
    }

    pub fn set_discovering(&self, discovering: bool) -> zbus::Result<()> {
        let Some(adapter) = self.adapter()? else {
            return Ok(());
        };
        if discovering {
            adapter.start_discovery()
        } else {
            adapter.stop_discovery()
        }
    }

    fn device(&self, device: &Device) -> zbus::Result<DeviceProxyBlocking<'static>> {
        DeviceProxyBlocking::builder(&self.bus)
            .path(device.path.clone())?
            .build()
    }

    pub fn connect(&self, device: &Device) -> zbus::Result<()> {
        self.device(device)?.connect()
    }

    pub fn disconnect(&self, device: &Device) -> zbus::Result<()> {
        self.device(device)?.disconnect()
    }

    pub fn forget(&self, device: &Device) -> zbus::Result<()> {
        match self.adapter()? {
            Some(adapter) => adapter.remove_device(&device.path),
            None => Ok(()),
        }
    }

    pub fn pair(&self, device: &Device) -> zbus::Result<()> {
        let proxy = self.device(device)?;
        proxy.pair()?;
        proxy.set_trusted(true)?;
        proxy.connect()
    }
}
