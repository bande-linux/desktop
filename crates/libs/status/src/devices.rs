use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use bande_bluetooth::{Bluetooth as BlueZ, adapter_in_kernel, shown_change};
use bande_inbox::{Inbox, Outbox};
use bande_network::{NetworkManager, set_strength, strength_signal, wifi_in_kernel};
use serde::Deserialize;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::names::BusName;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedObjectPath, Type};
use zbus::{MatchRule, Message, message};

use crate::session::{Ending, Inhibitor, Refusal};
use crate::status::{Battery, Bluetooth, Gadget, Power, Reading, Wifi};
use crate::{Command, Update};

const SOURCES: [(Source, &str); 3] = [
    (Source::Battery, "org.freedesktop.UPower"),
    (Source::Wifi, "org.freedesktop.NetworkManager"),
    (Source::Bluetooth, "org.bluez"),
];
const BUS: &str = "org.freedesktop.DBus";
const COMPOSITOR_UNIT: &str = "bande-compositor.service";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const UPOWER_CHARGING: u32 = 1;
const UPOWER_FULL: u32 = 4;
const UPOWER_BATTERY: u32 = 2;
const KERNEL_POWER_SUPPLIES: &str = "/sys/class/power_supply";

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/devices/DisplayDevice",
    gen_async = false,
    blocking_name = "DisplayDevice"
)]
trait UPowerDevice {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;

    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
}

#[derive(Deserialize, Type)]
struct LogindInhibitor {
    what: String,
    who: String,
    why: String,
    mode: String,
    _uid: u32,
    _pid: u32,
}

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1",
    gen_async = false,
    blocking_name = "Login"
)]
trait LoginManager {
    fn suspend(&self, interactive: bool) -> zbus::Result<()>;

    fn reboot(&self, interactive: bool) -> zbus::Result<()>;

    fn power_off(&self, interactive: bool) -> zbus::Result<()>;

    fn can_suspend(&self) -> zbus::Result<String>;

    fn can_reboot(&self) -> zbus::Result<String>;

    fn can_power_off(&self) -> zbus::Result<String>;

    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;

    fn list_inhibitors(&self) -> zbus::Result<Vec<LogindInhibitor>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Sleep,
    Restart,
    ShutDown,
}

impl PowerAction {
    pub fn ending(self) -> Option<Ending> {
        match self {
            Self::Sleep => None,
            Self::Restart => Some(Ending::Restart),
            Self::ShutDown => Some(Ending::ShutDown),
        }
    }
}

pub fn powers(system: &Connection) -> zbus::Result<Vec<PowerAction>> {
    let login = Login::new(system)?;
    let answers = [
        (PowerAction::Sleep, login.can_suspend()?),
        (PowerAction::Restart, login.can_reboot()?),
        (PowerAction::ShutDown, login.can_power_off()?),
    ];
    Ok(answers
        .into_iter()
        .filter(|(_, answer)| answer == "yes")
        .map(|(power, _)| power)
        .collect())
}

pub fn can_sleep(system: &Connection) -> bool {
    powers(system).is_ok_and(|powers| powers.contains(&PowerAction::Sleep))
}

pub fn hear_if_cannot_sleep() -> std::io::Result<Inbox<()>> {
    let answer = Inbox::new()?;
    let cannot_sleep = answer.outbox();
    thread::Builder::new()
        .name("can-sleep".to_owned())
        .spawn(move || {
            if !Connection::system().is_ok_and(|system| can_sleep(&system)) {
                cannot_sleep.send(());
            }
        })?;
    Ok(answer)
}

pub fn use_power(system: &Connection, power: PowerAction) -> zbus::Result<()> {
    let login = Login::new(system)?;
    match power {
        PowerAction::Sleep => login.suspend(false),
        PowerAction::Restart => login.reboot(false),
        PowerAction::ShutDown => login.power_off(false),
    }
}

fn seconds(value: i64) -> Option<Duration> {
    u64::try_from(value)
        .ok()
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

pub fn battery_of(state: u32, percent: f64, to_empty: i64, to_full: i64) -> Battery {
    let power = match state {
        UPOWER_FULL => Power::Full,
        UPOWER_CHARGING => Power::Charging {
            until_full: seconds(to_full),
        },
        _ if percent >= 100.0 => Power::Full,
        _ => Power::Battery {
            remaining: seconds(to_empty),
        },
    };
    Battery { percent, power }
}

pub fn battery_in_kernel() -> bool {
    let Ok(supplies) = std::fs::read_dir(KERNEL_POWER_SUPPLIES) else {
        return false;
    };
    supplies.flatten().any(|supply| {
        let read = |name: &str| std::fs::read_to_string(supply.path().join(name));
        read("type").is_ok_and(|kind| kind.trim() == "Battery")
            && read("scope")
                .ok()
                .is_none_or(|scope| scope.trim() != "Device")
    })
}

fn display_battery(system: &Connection) -> zbus::Result<Option<Battery>> {
    let device = DisplayDevice::builder(system)
        .cache_properties(CacheProperties::No)
        .build()?;
    if !device.is_present()? || device.kind()? != UPOWER_BATTERY {
        return Ok(None);
    }
    Ok(Some(battery_of(
        device.state().unwrap_or(0),
        device.percentage()?,
        device.time_to_empty().unwrap_or(0),
        device.time_to_full().unwrap_or(0),
    )))
}

pub fn battery(system: &Connection) -> Reading<Battery> {
    Reading::from_answer("UPower", display_battery(system), battery_in_kernel)
}

fn read_wifi(network: &NetworkManager) -> bande_network::Result<Option<Wifi>> {
    if !network.has_wifi()? {
        return Ok(None);
    }
    let enabled = network.wifi_enabled()?;
    let networks = if enabled {
        match network.networks() {
            Err(bande_network::Error::NoWifiDevice) => return Ok(None),
            networks => networks?,
        }
    } else {
        Vec::new()
    };
    Ok(Some(Wifi {
        enabled,
        networks,
        sign_in: matches!(network.sign_in_page(), Ok(Some(_))),
    }))
}

fn wifi(network: &NetworkManager) -> Reading<Wifi> {
    Reading::from_answer("NetworkManager", read_wifi(network), wifi_in_kernel)
}

fn bluetooth(bluez: &BlueZ) -> Reading<Bluetooth> {
    let radio = bluez.radio().map(|radio| {
        radio.map(|radio| Bluetooth {
            powered: radio.powered,
            devices: radio
                .paired
                .into_iter()
                .map(|device| Gadget {
                    address: device.address,
                    name: device.name,
                    connected: device.connected,
                    battery: device.battery,
                })
                .collect(),
        })
    });
    Reading::from_answer("BlueZ", radio, adapter_in_kernel)
}

pub type Devices = (Reading<Battery>, Reading<Wifi>, Reading<Bluetooth>);

#[derive(Debug, PartialEq)]
pub enum Control {
    Run(Command),
    Scan,
    Refresh(Source),
    Strength(String, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Battery,
    Wifi,
    Bluetooth,
}

pub(crate) fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

fn paired(bluez: &BlueZ, address: &str) -> Option<bande_bluetooth::Device> {
    bluez
        .radio()
        .ok()??
        .paired
        .into_iter()
        .find(|device| device.address == address)
}

fn run(system: &Connection, network: &NetworkManager, bluez: &BlueZ, command: Command) {
    match command {
        Command::SetWifi(on) => report("switch Wi-Fi", network.set_wifi_enabled(on)),
        Command::Join(ssid) => {
            let found = network
                .networks()
                .unwrap_or_default()
                .into_iter()
                .find(|candidate| candidate.ssid == ssid);
            if let Some(found) = found {
                crate::captive::join_network(network, &found);
            }
        }
        Command::SetBluetooth(on) => report("switch Bluetooth", bluez.set_powered(on)),
        Command::SignIn => match network.sign_in_page() {
            Ok(Some(page)) => crate::captive::open_page(&page),
            Ok(None) => {}
            Err(error) => tracing::warn!("cannot find the sign-in page: {error}"),
        },
        Command::ForgetGadget(address) => {
            if let Some(device) = paired(bluez, &address) {
                report("forget the device", bluez.forget(&device));
            }
        }
        Command::ToggleGadget(address) => {
            if let Some(device) = paired(bluez, &address) {
                let result = if device.connected {
                    bluez.disconnect(&device)
                } else {
                    bluez.connect(&device)
                };
                report("connect the device", result);
            }
        }
        Command::Sleep => report("sleep", sleep(system)),
        Command::LockScreen => report("lock the screen", lock_screen(system)),
        _ => {}
    }
}

fn own_session(system: &Connection, method: &str) -> zbus::Result<()> {
    let path = Login::new(system)?.get_session("auto")?;
    system.call_method(
        Some("org.freedesktop.login1"),
        path.as_str(),
        Some("org.freedesktop.login1.Session"),
        method,
        &(),
    )?;
    Ok(())
}

pub fn lock_screen(system: &Connection) -> zbus::Result<()> {
    own_session(system, "Lock")
}

pub fn sleep(system: &Connection) -> zbus::Result<()> {
    Login::new(system)?.suspend(true)
}

fn shutdown_inhibitors(login: &Login) -> zbus::Result<Vec<Inhibitor>> {
    Ok(login
        .list_inhibitors()?
        .into_iter()
        .filter(|held| held.mode == "block" && held.what.split(':').any(|what| what == "shutdown"))
        .map(|held| Inhibitor {
            who: held.who,
            why: held.why,
        })
        .collect())
}

fn stop_session() -> zbus::Result<()> {
    Connection::session()?.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StopUnit",
        &(COMPOSITOR_UNIT, "replace"),
    )?;
    Ok(())
}

fn turn_off(ending: Ending, insist: bool) -> zbus::Result<Result<(), Vec<Inhibitor>>> {
    let system = Connection::system()?;
    let login = Login::new(&system)?;
    if !insist {
        let inhibitors = shutdown_inhibitors(&login)?;
        if !inhibitors.is_empty() {
            return Ok(Err(inhibitors));
        }
    }
    match ending {
        Ending::Restart => login.reboot(true)?,
        _ => login.power_off(true)?,
    }
    Ok(Ok(()))
}

pub fn end_session(ending: Ending, insist: bool) -> Result<(), Refusal> {
    let requested = match ending {
        Ending::LogOut => stop_session().map(Ok),
        Ending::Restart | Ending::ShutDown => turn_off(ending, insist),
    };
    match requested {
        Ok(Ok(())) => Ok(()),
        Ok(Err(inhibitors)) => Err(Refusal::Inhibited(inhibitors)),
        Err(error) => {
            tracing::warn!("cannot end the session: {error}");
            Err(Refusal::Failed(match error {
                zbus::Error::MethodError(_, Some(reason), _) => reason,
                error => error.to_string(),
            }))
        }
    }
}

fn device_rules() -> zbus::Result<Vec<MatchRule<'static>>> {
    let mut rules = Vec::new();
    for (source, service) in SOURCES {
        let signals = MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(service)?;
        rules.push(match source {
            Source::Battery => signals.path(DISPLAY_DEVICE)?.build(),
            Source::Wifi | Source::Bluetooth => signals.build(),
        });
        rules.push(
            MatchRule::builder()
                .msg_type(message::Type::Signal)
                .sender(BUS)?
                .interface(BUS)?
                .member("NameOwnerChanged")?
                .add_arg(service)?
                .build(),
        );
    }
    Ok(rules)
}

type Owners = [Option<String>; SOURCES.len()];

fn change_of(message: &Message, owners: &mut Owners) -> Option<Control> {
    let header = message.header();
    if header.message_type() != message::Type::Signal {
        return None;
    }
    if header
        .interface()
        .is_none_or(|interface| interface.as_str() != BUS)
    {
        let sender = header.sender()?.as_str();
        let index = owners
            .iter()
            .position(|owner| owner.as_deref() == Some(sender))?;
        let source = SOURCES[index].0;
        if source == Source::Wifi
            && let Some(strength) = strength_signal(message)
        {
            let access_point = header.path()?.to_string();
            return strength.map(|strength| Control::Strength(access_point, strength));
        }
        if source == Source::Bluetooth && !shown_change(message) {
            return None;
        }
        return Some(Control::Refresh(source));
    }
    if header
        .member()
        .is_none_or(|member| member.as_str() != "NameOwnerChanged")
        || header.sender().is_none_or(|sender| sender.as_str() != BUS)
    {
        return None;
    }
    let (name, _, owner): (String, String, String) = message.body().deserialize().ok()?;
    let index = SOURCES.iter().position(|(_, service)| *service == name)?;
    owners[index] = (!owner.is_empty()).then_some(owner);
    Some(Control::Refresh(SOURCES[index].0))
}

fn refresh_all(controls: &Sender<Control>) -> bool {
    SOURCES
        .iter()
        .all(|(source, _)| controls.send(Control::Refresh(*source)).is_ok())
}

fn watch_devices(system: &Connection, controls: &Sender<Control>) -> zbus::Result<()> {
    let messages = MessageIterator::from(system);
    let bus = DBusProxy::new(system)?;
    for rule in device_rules()? {
        bus.add_match_rule(rule)?;
    }
    let mut owners: Owners = SOURCES.map(|(_, service)| {
        BusName::try_from(service)
            .ok()
            .and_then(|name| bus.get_name_owner(name).ok())
            .map(|owner| owner.to_string())
    });
    if !refresh_all(controls) {
        return Ok(());
    }
    for message in messages.flatten() {
        if let Some(control) = change_of(&message, &mut owners)
            && controls.send(control).is_err()
        {
            break;
        }
    }
    Ok(())
}

fn run_controls(system: &Connection, controls: &Receiver<Control>, outbox: &Outbox<Update>) {
    let network = NetworkManager::new(system);
    let bluez = BlueZ::new(system);
    let mut devices: Devices = (battery(system), wifi(&network), bluetooth(&bluez));
    if !outbox.send(Update::Devices(devices.clone())) {
        return;
    }
    let mut shown = devices.clone();
    while let Ok(first) = controls.recv() {
        let mut stale = Vec::new();
        for control in std::iter::once(first).chain(controls.try_iter()) {
            match control {
                Control::Run(command) => run(system, &network, &bluez, command),
                Control::Scan => report("scan for networks", network.scan()),
                Control::Refresh(source) => {
                    stale.push(source);
                    continue;
                }
                Control::Strength(access_point, strength) => {
                    if let Some(wifi) = devices.1.present_mut() {
                        set_strength(&mut wifi.networks, &access_point, strength);
                    }
                    continue;
                }
            }
            stale.extend(SOURCES.map(|(source, _)| source));
        }
        if stale.contains(&Source::Battery) {
            devices.0 = battery(system);
        }
        if stale.contains(&Source::Wifi) {
            devices.1 = wifi(&network);
        }
        if stale.contains(&Source::Bluetooth) {
            devices.2 = bluetooth(&bluez);
        }
        if !looks_alike(&devices, &shown) {
            shown = devices.clone();
            if !outbox.send(Update::Devices(devices.clone())) {
                return;
            }
        }
    }
}

fn looks_alike(devices: &Devices, shown: &Devices) -> bool {
    let wifi = match (&devices.1, &shown.1) {
        (Reading::Present(wifi), Reading::Present(shown)) => wifi.looks_like(shown),
        (wifi, shown) => wifi == shown,
    };
    wifi && devices.0 == shown.0 && devices.2 == shown.2
}

pub fn follow_devices(system: &Connection, outbox: Outbox<Update>) -> Sender<Control> {
    let (controls, received) = mpsc::channel();
    let (watching, refreshes) = (system.clone(), controls.clone());
    thread::spawn(move || {
        if let Err(error) = watch_devices(&watching, &refreshes) {
            tracing::warn!("battery, Wi-Fi and Bluetooth changes are not followed: {error}");
            refresh_all(&refreshes);
        }
    });
    let system = system.clone();
    thread::spawn(move || run_controls(&system, &received, &outbox));
    controls
}

pub enum SoundControl {
    SetMuted(bool),
    UseOutput(String),
    Nudge(f32),
}
