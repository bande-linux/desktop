use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use unbound_audio::{Audio, Level, Snapshot};
use unbound_bluetooth::Bluetooth as BlueZ;
use unbound_inbox::Outbox;
use unbound_network::NetworkManager;
use zbus::blocking::fdo::DBusProxy;
use zbus::blocking::{Connection, MessageIterator};
use zbus::names::BusName;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;
use zbus::{MatchRule, Message, message};

use crate::session::Ending;
use crate::status::{Battery, Bluetooth, Gadget, Output, Power, Sound, Wifi};
use crate::{Command, Update};

const SOURCES: [(Source, &str); 3] = [
    (Source::Battery, "org.freedesktop.UPower"),
    (Source::Wifi, "org.freedesktop.NetworkManager"),
    (Source::Bluetooth, "org.bluez"),
];
const BUS: &str = "org.freedesktop.DBus";
const UPOWER_CHARGING: u32 = 1;
const UPOWER_FULL: u32 = 4;
const UPOWER_BATTERY: u32 = 2;

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerAction {
    Sleep,
    Restart,
    ShutDown,
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

fn battery(system: &Connection) -> Option<Battery> {
    let device = DisplayDevice::builder(system)
        .cache_properties(CacheProperties::No)
        .build()
        .ok()?;
    if !device.is_present().ok()? || device.kind().ok()? != UPOWER_BATTERY {
        return None;
    }
    Some(battery_of(
        device.state().unwrap_or(0),
        device.percentage().ok()?,
        device.time_to_empty().unwrap_or(0),
        device.time_to_full().unwrap_or(0),
    ))
}

fn wifi(network: &NetworkManager) -> Option<Wifi> {
    if !network.has_wifi().ok()? {
        return None;
    }
    let enabled = network.wifi_enabled().ok()?;
    Some(Wifi {
        enabled,
        networks: if enabled {
            network.networks().unwrap_or_default()
        } else {
            Vec::new()
        },
        sign_in: matches!(network.sign_in_page(), Ok(Some(_))),
    })
}

fn bluetooth(bluez: &BlueZ) -> Option<Bluetooth> {
    if !bluez.available().ok()? {
        return None;
    }
    let powered = bluez.powered().ok()?;
    Some(Bluetooth {
        powered,
        devices: bluez
            .paired_devices()
            .unwrap_or_default()
            .into_iter()
            .map(|device| Gadget {
                address: device.address,
                name: device.name,
                connected: device.connected,
                battery: device.battery,
            })
            .collect(),
    })
}

pub type Devices = (Option<Battery>, Option<Wifi>, Option<Bluetooth>);

pub enum Control {
    Run(Command),
    Scan,
    Refresh(Source),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Battery,
    Wifi,
    Bluetooth,
}

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

fn paired(bluez: &BlueZ, address: &str) -> Option<unbound_bluetooth::Device> {
    bluez
        .paired_devices()
        .unwrap_or_default()
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
                report("join the network", network.join(&found, None).map(|_| ()));
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
        Command::LockScreen => report("lock the screen", own_session(system, "Lock")),
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

pub fn sleep(system: &Connection) -> zbus::Result<()> {
    Login::new(system)?.suspend(true)
}

pub fn end_session(system: &Connection, ending: Ending) -> zbus::Result<()> {
    let login = Login::new(system)?;
    match ending {
        Ending::Restart => login.reboot(true),
        Ending::ShutDown => login.power_off(true),
        Ending::LogOut => own_session(system, "Terminate"),
    }
}

fn device_rules() -> zbus::Result<Vec<MatchRule<'static>>> {
    let mut rules = Vec::new();
    for (_, service) in SOURCES {
        rules.push(
            MatchRule::builder()
                .msg_type(message::Type::Signal)
                .sender(service)?
                .build(),
        );
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

fn changed_source(message: &Message, owners: &mut Owners) -> Option<Source> {
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
        return Some(SOURCES[index].0);
    }
    if header
        .member()
        .is_none_or(|member| member.as_str() != "NameOwnerChanged")
    {
        return None;
    }
    let (name, _, owner): (String, String, String) = message.body().deserialize().ok()?;
    let index = SOURCES.iter().position(|(_, service)| *service == name)?;
    owners[index] = (!owner.is_empty()).then_some(owner);
    Some(SOURCES[index].0)
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
        if let Some(source) = changed_source(&message, &mut owners)
            && controls.send(Control::Refresh(source)).is_err()
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
        if devices != shown {
            shown = devices.clone();
            if !outbox.send(Update::Devices(devices.clone())) {
                return;
            }
        }
    }
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

pub fn sound_of(snapshot: &Snapshot) -> Option<Sound> {
    let output = snapshot.default_output()?;
    Some(Sound {
        volume: output.level.volume,
        muted: output.level.muted,
        outputs: snapshot
            .outputs
            .iter()
            .map(|device| Output {
                name: device.name.clone(),
                description: device.description.clone(),
            })
            .collect(),
        default_output: snapshot.default_output.clone(),
    })
}

fn control_sound(audio: &Audio, control: SoundControl) -> Result<(), unbound_audio::Error> {
    let snapshot = audio.snapshot();
    match control {
        SoundControl::UseOutput(name) => match snapshot.device(&name) {
            Some(device) => audio.set_default(device),
            None => Ok(()),
        },
        SoundControl::SetMuted(muted) => match snapshot.default_output() {
            Some(output) => audio.set_muted(output, muted),
            None => Ok(()),
        },
        SoundControl::Nudge(delta) => match snapshot.default_output() {
            Some(output) => audio.set_level(
                output,
                Level {
                    volume: (output.level.volume + delta).clamp(0.0, 1.0),
                    muted: false,
                },
            ),
            None => Ok(()),
        },
    }
}

fn run_sound(audio: &Audio, outbox: &Outbox<Update>) {
    let mut generation = None;
    let mut capturing = None;
    loop {
        let snapshot = audio.changed_since(generation);
        generation = Some(snapshot.generation);
        if !outbox.send(Update::Sound(sound_of(&snapshot))) {
            return;
        }
        if capturing.as_ref() != Some(&snapshot.capturing) {
            capturing = Some(snapshot.capturing.clone());
            if !outbox.send(Update::Capturing(snapshot.capturing)) {
                return;
            }
        }
    }
}

pub(crate) fn follow_sound(audio: Arc<Audio>, outbox: Outbox<Update>) -> Sender<SoundControl> {
    let (controls, received) = mpsc::channel::<SoundControl>();
    let controlled = Arc::clone(&audio);
    thread::spawn(move || {
        for control in received {
            report("change the sound", control_sound(&controlled, control));
        }
    });
    thread::spawn(move || run_sound(&audio, &outbox));
    controls
}

#[cfg(test)]
mod tests {
    use zbus::Message;

    use super::{Owners, Source, changed_source};

    fn changed(sender: &str) -> Message {
        Message::signal(
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .unwrap()
        .sender(sender)
        .unwrap()
        .build(&())
        .unwrap()
    }

    fn owner_changed(name: &str, owner: &str) -> Message {
        Message::signal(
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
        )
        .unwrap()
        .sender("org.freedesktop.DBus")
        .unwrap()
        .build(&(name, "", owner))
        .unwrap()
    }

    #[test]
    fn a_signal_refreshes_only_the_service_that_sent_it() {
        let mut owners: Owners = [Some(":1.5".to_owned()), Some(":1.6".to_owned()), None];
        assert_eq!(
            changed_source(&changed(":1.5"), &mut owners),
            Some(Source::Battery)
        );
        assert_eq!(
            changed_source(&changed(":1.6"), &mut owners),
            Some(Source::Wifi)
        );
        assert_eq!(changed_source(&changed(":1.7"), &mut owners), None);
    }

    #[test]
    fn a_service_that_starts_or_restarts_is_followed_under_its_new_name() {
        let mut owners: Owners = [None, None, None];
        assert_eq!(
            changed_source(&owner_changed("org.bluez", ":1.9"), &mut owners),
            Some(Source::Bluetooth)
        );
        assert_eq!(
            changed_source(&changed(":1.9"), &mut owners),
            Some(Source::Bluetooth)
        );
        assert_eq!(
            changed_source(&owner_changed("org.bluez", ""), &mut owners),
            Some(Source::Bluetooth)
        );
        assert_eq!(changed_source(&changed(":1.9"), &mut owners), None);
        assert_eq!(
            changed_source(&owner_changed("org.example.Other", ":1.4"), &mut owners),
            None
        );
    }
}
