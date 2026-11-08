use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use std::sync::mpsc::RecvTimeoutError;

use bande_audio::{Audio, Direction, Level, Meter};
use bande_bluetooth::{
    Bluetooth, PairingPrompt, adapter_in_kernel, change_rules, register_agent, shown_change,
};
use bande_bus::Reading;
use bande_network::{
    ConnectionChange, JoinOutcome, ManualIpv4, NetworkManager, Security, WifiLink, set_strength,
    strength_signal, wifi_in_kernel,
};
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, Message, message};

use bande_settings_panes::PaneId;

use crate::failure::pane_failed;
use crate::follow::Update;
use crate::follow::{Updates, spawn};
use crate::state::{Answer, BluetoothState, Joined, NetworkState, Question, WifiState};

pub(crate) const SETTLE: Duration = Duration::from_millis(200);
const METER_INTERVAL: Duration = Duration::from_millis(50);
const ANSWER_WAIT: Duration = Duration::from_secs(60);
const NETWORK_MANAGER: &str = "/org/freedesktop/NetworkManager";
const NETWORK_MANAGER_SERVICE: &str = "org.freedesktop.NetworkManager";
const BUS: &str = "org.freedesktop.DBus";

#[derive(Debug, Clone, PartialEq)]
pub enum AudioCommand {
    Volume(Direction, f32),
    Balance(f32),
    Mute(Direction, bool),
    Default(Direction, String),
    App(String, f32),
    Feedback,
    Listen(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum NetworkCommand {
    Power(bool),
    Scan,
    Join(bande_network::Network, Option<String>),
    JoinHidden(Vec<u8>, Security, Option<String>),
    Forget(Vec<u8>),
    ChangeConnection(PaneId, String, ConnectionChange),
    SignIn,
    ConfigureEthernet(Option<ManualIpv4>),
    ConnectVpn(String),
    DisconnectVpn(String),
    RemoveVpn(String),
    Watch(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BluetoothCommand {
    Power(bool),
    Discover(bool),
    Connect(bande_bluetooth::Device),
    Disconnect(bande_bluetooth::Device),
    Pair(bande_bluetooth::Device),
    Forget(bande_bluetooth::Device),
}

fn report<E: std::fmt::Display>(
    updates: &Updates,
    pane: PaneId,
    what: &str,
    result: Result<(), E>,
) {
    if let Err(error) = result {
        updates.send(pane_failed(pane, what, error));
    }
}

fn same_kind(a: &AudioCommand, b: &AudioCommand) -> bool {
    match (a, b) {
        (AudioCommand::Volume(a, _), AudioCommand::Volume(b, _)) => a == b,
        (AudioCommand::App(a, _), AudioCommand::App(b, _)) => a == b,
        (AudioCommand::Balance(_), AudioCommand::Balance(_)) => true,
        _ => false,
    }
}

fn audio_device(audio: &Audio, direction: Direction) -> Option<bande_audio::Device> {
    let snapshot = audio.snapshot();
    match direction {
        Direction::Output => snapshot.default_output().cloned(),
        Direction::Input => snapshot.default_input().cloned(),
    }
}

fn run_audio(audio: &Audio, command: AudioCommand) -> Result<(), bande_audio::Error> {
    let device = |direction| audio_device(audio, direction).ok_or(bande_audio::Error::NoSuchDevice);
    match command {
        AudioCommand::Volume(direction, volume) => audio.set_level(
            &device(direction)?,
            Level {
                volume,
                muted: false,
            },
        ),
        AudioCommand::Balance(balance) => audio.set_balance(&device(Direction::Output)?, balance),
        AudioCommand::Mute(direction, muted) => audio.set_muted(&device(direction)?, muted),
        AudioCommand::Default(_, name) => {
            let snapshot = audio.snapshot();
            let device = snapshot
                .device(&name)
                .ok_or(bande_audio::Error::NoSuchDevice)?;
            audio.set_default(device)
        }
        AudioCommand::App(name, volume) => {
            let snapshot = audio.snapshot();
            let app = snapshot
                .apps
                .iter()
                .find(|app| app.name == name)
                .ok_or(bande_audio::Error::NoSuchDevice)?;
            audio.set_app_level(
                app,
                Level {
                    volume,
                    muted: false,
                },
            )
        }
        AudioCommand::Feedback | AudioCommand::Listen(_) => Ok(()),
    }
}

struct Listening {
    connection: u64,
    input: String,
    meter: Meter,
}

fn listen(audio: &Audio, listening: Option<Listening>) -> Option<Listening> {
    let snapshot = audio.snapshot();
    let input = snapshot.default_input()?;
    let current = |listening: &Listening| {
        listening.connection == snapshot.connection && listening.input == input.name
    };
    if let Some(listening) = listening.filter(current) {
        return Some(listening);
    }
    let meter = audio
        .meter(input)
        .inspect_err(|error| tracing::warn!("no input level: {error}"))
        .ok()?;
    Some(Listening {
        connection: snapshot.connection,
        input: input.name.clone(),
        meter,
    })
}

pub fn follow_audio(session: &Connection, updates: Updates) -> Sender<AudioCommand> {
    let (sender, receiver) = mpsc::channel::<AudioCommand>();
    let session = session.clone();
    spawn("audio", move || {
        let audio = match Audio::start() {
            Ok(audio) => Arc::new(audio),
            Err(error) => {
                tracing::warn!("no sound settings: {error}");
                return;
            }
        };
        let watched = Arc::clone(&audio);
        let feed = updates.clone();
        spawn("audio-follow", move || {
            let mut generation = None;
            loop {
                let snapshot = watched.changed_since(generation);
                generation = Some(snapshot.generation);
                if !feed.send(Update::Audio(Some(snapshot))) {
                    break;
                }
            }
        });
        let mut wanted = false;
        let mut listening: Option<Listening> = None;
        loop {
            let first = if wanted {
                match receiver.recv_timeout(METER_INTERVAL) {
                    Ok(command) => Some(command),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match receiver.recv() {
                    Ok(command) => Some(command),
                    Err(_) => break,
                }
            };
            let mut commands: Vec<AudioCommand> = first.into_iter().collect();
            for command in receiver.try_iter() {
                commands.retain(|queued| !same_kind(queued, &command));
                commands.push(command);
            }
            for command in commands {
                match command {
                    AudioCommand::Listen(on) => wanted = on,
                    AudioCommand::Feedback => {
                        bande_sound::play(&session, bande_sound::Sound::VolumeChange.name());
                    }
                    _ => {}
                }
                report(
                    &updates,
                    PaneId::Sound,
                    "sound-change-failed",
                    run_audio(&audio, command),
                );
            }
            listening = if wanted {
                listen(&audio, listening.take())
            } else {
                None
            };
            if let Some(listening) = &listening
                && !updates.send(Update::InputLevel(listening.meter.take_peak()))
            {
                break;
            }
        }
    });
    sender
}

pub(crate) fn signals<T: Send + 'static>(
    bus: &Connection,
    rule: MatchRule<'static>,
    changed: Sender<T>,
    classify: impl Fn(&Message) -> Option<T> + Send + 'static,
    wanted: impl Fn() -> bool + Send + 'static,
) {
    let bus = bus.clone();
    spawn("signals", move || {
        let iterator = match MessageIterator::for_match_rule(rule, &bus, None) {
            Ok(iterator) => iterator,
            Err(error) => {
                tracing::warn!("cannot subscribe to signals: {error}");
                return;
            }
        };
        for message in iterator.flatten() {
            if !wanted() {
                break;
            }
            if let Some(change) = classify(&message)
                && changed.send(change).is_err()
            {
                break;
            }
        }
    });
}

pub(crate) fn follow_state(
    name: &str,
    updates: Updates,
    changed: Receiver<()>,
    read: impl Fn() -> Vec<Update> + Send + 'static,
) {
    spawn(name, move || {
        let publish = || read().into_iter().all(|update| updates.send(update));
        if !publish() {
            return;
        }
        while changed.recv().is_ok() {
            std::thread::sleep(SETTLE);
            changed.try_iter().count();
            if !publish() {
                break;
            }
        }
    });
}

fn wifi_state(network: &NetworkManager) -> bande_network::Result<Option<WifiState>> {
    if !network.has_wifi()? {
        return Ok(None);
    }
    let enabled = network.wifi_enabled()?;
    let (networks, link) = if enabled {
        let networks = match network.networks() {
            Err(bande_network::Error::NoWifiDevice) => return Ok(None),
            networks => networks?,
        };
        (networks, network.wifi_link().unwrap_or_default())
    } else {
        (Vec::new(), WifiLink::default())
    };
    Ok(Some(WifiState {
        enabled,
        networks,
        known: network.known_networks()?,
        link,
        sign_in: enabled && matches!(network.sign_in_page(), Ok(Some(_))),
    }))
}

fn read_wifi(network: &NetworkManager) -> Reading<WifiState> {
    Reading::from_answer("NetworkManager", wifi_state(network), wifi_in_kernel)
}

enum NetworkChange {
    Read,
    Strength(String, u8),
}

fn network_change(message: &Message) -> Option<NetworkChange> {
    match strength_signal(message) {
        None => Some(NetworkChange::Read),
        Some(strength) => Some(NetworkChange::Strength(
            message.header().path()?.to_string(),
            strength?,
        )),
    }
}

fn follow_wifi(reader: NetworkManager, updates: Updates, changes: Receiver<NetworkChange>) {
    spawn("network", move || {
        let mut wifi = read_wifi(&reader);
        let publish = |wifi: &Reading<WifiState>| {
            updates.send(Update::Wifi(wifi.clone()))
                && updates.send(Update::Network(read_network(&reader)))
        };
        if !publish(&wifi) {
            return;
        }
        while let Ok(first) = changes.recv() {
            std::thread::sleep(SETTLE);
            let batch: Vec<NetworkChange> =
                std::iter::once(first).chain(changes.try_iter()).collect();
            if batch
                .iter()
                .any(|change| matches!(change, NetworkChange::Read))
            {
                wifi = read_wifi(&reader);
                if !publish(&wifi) {
                    break;
                }
                continue;
            }
            let mut moved = false;
            if let Some(state) = wifi.present_mut() {
                for change in batch {
                    if let NetworkChange::Strength(access_point, strength) = change {
                        moved |= set_strength(&mut state.networks, &access_point, strength);
                    }
                }
            }
            if moved && !updates.send(Update::Wifi(wifi.clone())) {
                break;
            }
        }
    });
}

fn read_network(network: &NetworkManager) -> NetworkState {
    NetworkState {
        ethernet: network.ethernet().ok().flatten(),
        vpns: network.vpns().unwrap_or_default(),
    }
}

fn rule<T>(build: impl FnOnce() -> zbus::Result<T>) -> Option<T> {
    build()
        .inspect_err(|error| tracing::warn!("invalid match rule: {error}"))
        .ok()
}

pub fn follow_network(system: &Connection, updates: Updates) -> Sender<NetworkCommand> {
    let (changed, changes) = mpsc::channel();
    if let Some(rule) = rule(|| {
        Ok(MatchRule::builder()
            .msg_type(message::Type::Signal)
            .path(NETWORK_MANAGER)?
            .build())
    }) {
        signals(system, rule, changed.clone(), network_change, || true);
    }
    if let Some(rule) = rule(|| {
        Ok(MatchRule::builder()
            .msg_type(message::Type::Signal)
            .sender(BUS)?
            .interface(BUS)?
            .member("NameOwnerChanged")?
            .add_arg(NETWORK_MANAGER_SERVICE)?
            .build())
    }) {
        signals(system, rule, changed.clone(), network_change, || true);
    }
    follow_wifi(NetworkManager::new(system), updates.clone(), changes);
    let (sender, receiver) = mpsc::channel();
    let network = NetworkManager::new(system);
    let bus = system.clone();
    spawn("network-worker", move || {
        let watching = Arc::new(AtomicU64::new(0));
        for command in receiver {
            match command {
                NetworkCommand::Watch(on) => {
                    let generation = watching.fetch_add(1, Ordering::Relaxed) + 1;
                    let every_change = rule(|| {
                        Ok(MatchRule::builder()
                            .msg_type(message::Type::Signal)
                            .path_namespace(NETWORK_MANAGER)?
                            .build())
                    });
                    if on && let Some(every_change) = every_change {
                        let current = Arc::clone(&watching);
                        signals(
                            &bus,
                            every_change,
                            changed.clone(),
                            network_change,
                            move || current.load(Ordering::Relaxed) == generation,
                        );
                        let _ = changed.send(NetworkChange::Read);
                    }
                }
                NetworkCommand::Power(on) => report(
                    &updates,
                    PaneId::Wifi,
                    "wifi-power-failed",
                    network.set_wifi_enabled(on),
                ),
                NetworkCommand::ConfigureEthernet(manual) => {
                    report(
                        &updates,
                        PaneId::Network,
                        "network-change-failed",
                        network.configure_ethernet(manual.as_ref()),
                    );
                }
                NetworkCommand::ConnectVpn(path) => report(
                    &updates,
                    PaneId::Network,
                    "vpn-connect-failed",
                    network.connect_vpn(&path),
                ),
                NetworkCommand::RemoveVpn(path) => report(
                    &updates,
                    PaneId::Network,
                    "vpn-remove-failed",
                    network.remove_vpn(&path),
                ),
                NetworkCommand::DisconnectVpn(path) => {
                    report(
                        &updates,
                        PaneId::Network,
                        "vpn-disconnect-failed",
                        network.disconnect_vpn(&path),
                    );
                }
                NetworkCommand::Forget(ssid) => report(
                    &updates,
                    PaneId::Wifi,
                    "wifi-forget-failed",
                    network.forget(&ssid),
                ),
                NetworkCommand::SignIn => match network.sign_in_page() {
                    Ok(Some(page)) => bande_status::captive::open_page(&page),
                    Ok(None) => {}
                    Err(error) => tracing::warn!("cannot find the sign-in page: {error}"),
                },
                NetworkCommand::ChangeConnection(pane, path, change) => {
                    report(
                        &updates,
                        pane,
                        "network-change-failed",
                        network.change_connection(&path, &change),
                    );
                }
                NetworkCommand::Scan => {
                    if let Err(error) = network.scan() {
                        tracing::debug!("scan not started: {error}");
                    }
                }
                NetworkCommand::Join(target, password) => {
                    let outcome = joined(network.join(&target, password.as_deref()));
                    updates.send(Update::Joined(target.ssid.clone(), outcome));
                }
                NetworkCommand::JoinHidden(ssid, security, password) => {
                    let outcome = joined(network.join_hidden(&ssid, security, password.as_deref()));
                    updates.send(Update::Joined(ssid, outcome));
                }
            }
        }
    });
    sender
}

fn joined(result: bande_network::Result<JoinOutcome>) -> Joined {
    match result {
        Ok(JoinOutcome::Joined) => Joined::Joined,
        Ok(JoinOutcome::WrongPassword) | Err(bande_network::Error::PasswordRequired) => {
            Joined::WrongPassword
        }
        Ok(failed @ JoinOutcome::Failed { .. }) => Joined::Failed(failed.to_string()),
        Err(error) => Joined::Failed(error.to_string()),
    }
}

fn read_bluetooth(bluetooth: &Bluetooth) -> Reading<BluetoothState> {
    Reading::from_answer("BlueZ", bluetooth.radio(), adapter_in_kernel)
}

struct Prompt {
    updates: Updates,
    answers: Mutex<Receiver<Answer>>,
}

impl Prompt {
    fn ask(&self, question: Question) -> Option<Answer> {
        let answers = self.answers.lock().unwrap_or_else(PoisonError::into_inner);
        answers.try_iter().count();
        self.updates.send(Update::Pairing(Some(question)));
        let answer = answers.recv_timeout(ANSWER_WAIT).ok();
        self.updates.send(Update::Pairing(None));
        answer
    }
}

impl PairingPrompt for Prompt {
    fn confirm_passkey(&self, device: &str, passkey: u32) -> bool {
        self.ask(Question::Confirm {
            device: device.to_owned(),
            code: format!("{passkey:06}"),
        }) == Some(Answer::Accept)
    }

    fn show_code(&self, device: &str, code: &str) {
        self.updates.send(Update::Pairing(Some(Question::Show {
            device: device.to_owned(),
            code: code.to_owned(),
        })));
    }

    fn ask_pin_code(&self, device: &str) -> Option<String> {
        match self.ask(Question::Pin {
            device: device.to_owned(),
        })? {
            Answer::Pin(pin) => Some(pin),
            Answer::Accept | Answer::Decline => None,
        }
    }

    fn authorize(&self, device: &str) -> bool {
        self.ask(Question::Authorize {
            device: device.to_owned(),
        }) == Some(Answer::Accept)
    }

    fn dismiss(&self) {
        self.updates.send(Update::Pairing(None));
    }
}

pub fn follow_bluetooth(
    system: &Connection,
    updates: Updates,
) -> (Sender<BluetoothCommand>, Sender<Answer>) {
    let (changed, changes) = mpsc::channel();
    let (adapters, properties): (Vec<MatchRule<'static>>, Vec<MatchRule<'static>>) =
        rule(change_rules)
            .unwrap_or_default()
            .into_iter()
            .partition(|rule| rule.interface().is_some());
    for rule in adapters {
        signals(system, rule, changed.clone(), |_| Some(()), || true);
    }
    let reader = Bluetooth::new(system);
    follow_state("bluetooth", updates.clone(), changes, move || {
        vec![Update::Bluetooth(read_bluetooth(&reader))]
    });
    let (answer, answers) = mpsc::channel();
    let prompt = Arc::new(Prompt {
        updates: updates.clone(),
        answers: Mutex::new(answers),
    });
    if let Err(error) = register_agent(system, prompt) {
        tracing::warn!("cannot pair from Settings: {error}");
    }
    let (sender, receiver) = mpsc::channel();
    let bluetooth = Bluetooth::new(&crate::follow::waiting_system(system));
    let bus = system.clone();
    spawn("bluetooth-worker", move || {
        let shown = Arc::new(AtomicU64::new(0));
        for command in receiver {
            let (what, result) = match command {
                BluetoothCommand::Power(on) => {
                    ("bluetooth-power-failed", bluetooth.set_powered(on))
                }
                BluetoothCommand::Discover(on) => {
                    let generation = shown.fetch_add(1, Ordering::Relaxed) + 1;
                    if on {
                        for rule in &properties {
                            let current = Arc::clone(&shown);
                            signals(
                                &bus,
                                rule.clone(),
                                changed.clone(),
                                |message| shown_change(message).then_some(()),
                                move || current.load(Ordering::Relaxed) == generation,
                            );
                        }
                        let _ = changed.send(());
                    }
                    if let Err(error) = bluetooth.set_discovering(on) {
                        tracing::debug!("discovery not changed: {error}");
                    }
                    continue;
                }
                BluetoothCommand::Connect(device) => {
                    ("bluetooth-connect-failed", bluetooth.connect(&device))
                }
                BluetoothCommand::Disconnect(device) => {
                    ("bluetooth-disconnect-failed", bluetooth.disconnect(&device))
                }
                BluetoothCommand::Forget(device) => {
                    ("bluetooth-forget-failed", bluetooth.forget(&device))
                }
                BluetoothCommand::Pair(device) => {
                    let result = bluetooth.pair(&device);
                    updates.send(Update::Pairing(None));
                    ("bluetooth-pair-failed", result)
                }
            };
            report(&updates, PaneId::Bluetooth, what, result);
        }
    });
    (sender, answer)
}
