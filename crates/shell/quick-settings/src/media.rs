use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use bande_inbox::Outbox;
use zbus::blocking::fdo::{DBusProxy, PropertiesProxy};
use zbus::blocking::{Connection, MessageIterator};
use zbus::names::InterfaceName;
use zbus::zvariant::{ObjectPath, OwnedValue};
use zbus::{MatchRule, message};

pub const PLAYER_PREFIX: &str = "org.mpris.MediaPlayer2.";
const ROOT_INTERFACE: &str = "org.mpris.MediaPlayer2";
const PLAYER_INTERFACE: &str = "org.mpris.MediaPlayer2.Player";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Playback {
    Playing,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Player {
    pub bus_name: String,
    pub identity: String,
    pub desktop_entry: Option<String>,
    pub title: String,
    pub artist: String,
    pub art_url: Option<String>,
    pub track: Option<String>,
    pub playback: Playback,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_go_next: bool,
    pub can_go_previous: bool,
    pub can_seek: bool,
    pub can_raise: bool,
    pub position: Option<Duration>,
    pub length: Option<Duration>,
    pub rate: f64,
    pub sampled: Instant,
}

type Properties = HashMap<String, OwnedValue>;

fn text(properties: &Properties, key: &str) -> Option<String> {
    properties
        .get(key)?
        .downcast_ref::<&str>()
        .ok()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn flag(properties: &Properties, key: &str) -> bool {
    properties
        .get(key)
        .and_then(|value| value.downcast_ref::<bool>().ok())
        .unwrap_or(false)
}

fn micros(properties: &Properties, key: &str) -> Option<Duration> {
    let value = properties.get(key)?;
    let micros = value
        .downcast_ref::<i64>()
        .ok()
        .or_else(|| {
            value
                .downcast_ref::<u64>()
                .ok()
                .and_then(|micros| i64::try_from(micros).ok())
        })
        .or_else(|| value.downcast_ref::<i32>().ok().map(i64::from))?;
    u64::try_from(micros).ok().map(Duration::from_micros)
}

fn artists(metadata: &Properties) -> String {
    let Some(value) = metadata.get("xesam:artist") else {
        return String::new();
    };
    if let Ok(single) = value.downcast_ref::<&str>() {
        return single.to_owned();
    }
    value
        .try_clone()
        .ok()
        .and_then(|owned| Vec::<String>::try_from(owned).ok())
        .unwrap_or_default()
        .join(", ")
}

fn track_id(metadata: &Properties) -> Option<String> {
    let value = metadata.get("mpris:trackid")?;
    value
        .downcast_ref::<ObjectPath<'_>>()
        .map(|path| path.as_str().to_owned())
        .or_else(|_| value.downcast_ref::<&str>().map(str::to_owned))
        .ok()
        .filter(|track| track != NO_TRACK)
}

impl Player {
    pub fn read(bus_name: &str, root: &Properties, player: &Properties, sampled: Instant) -> Self {
        let metadata: Properties = player
            .get("Metadata")
            .and_then(|value| value.try_clone().ok())
            .and_then(|owned| Properties::try_from(owned).ok())
            .unwrap_or_default();
        let playback = match text(player, "PlaybackStatus").as_deref() {
            Some("Playing") => Playback::Playing,
            Some("Paused") => Playback::Paused,
            _ => Playback::Stopped,
        };
        let identity = text(root, "Identity").unwrap_or_else(|| {
            let short = bus_name.trim_start_matches(PLAYER_PREFIX);
            short.split('.').next().unwrap_or(short).to_owned()
        });
        Self {
            bus_name: bus_name.to_owned(),
            identity,
            desktop_entry: text(root, "DesktopEntry"),
            title: text(&metadata, "xesam:title").unwrap_or_default(),
            artist: artists(&metadata),
            art_url: text(&metadata, "mpris:artUrl"),
            track: track_id(&metadata),
            playback,
            can_play: flag(player, "CanPlay"),
            can_pause: flag(player, "CanPause"),
            can_go_next: flag(player, "CanGoNext"),
            can_go_previous: flag(player, "CanGoPrevious"),
            can_seek: flag(player, "CanSeek"),
            can_raise: flag(root, "CanRaise"),
            position: micros(player, "Position"),
            length: micros(&metadata, "mpris:length").filter(|length| !length.is_zero()),
            rate: player
                .get("Rate")
                .and_then(|value| value.downcast_ref::<f64>().ok())
                .unwrap_or(1.0),
            sampled,
        }
    }

    pub fn position_at(&self, now: Instant) -> Option<Duration> {
        let position = self.position?;
        let elapsed = if self.playback == Playback::Playing {
            now.saturating_duration_since(self.sampled)
                .mul_f64(self.rate.max(0.0))
        } else {
            Duration::ZERO
        };
        let moved = position + elapsed;
        Some(self.length.map_or(moved, |length| moved.min(length)))
    }

    pub fn same_as(&self, other: &Self) -> bool {
        Self {
            sampled: other.sampled,
            position: other.position,
            ..self.clone()
        } == *other
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Players {
    players: Vec<Player>,
    recent: Vec<String>,
}

impl Players {
    pub fn update(&mut self, fresh: Vec<Player>) {
        for player in &fresh {
            let was_playing = self
                .players
                .iter()
                .find(|known| known.bus_name == player.bus_name)
                .is_some_and(|known| known.playback == Playback::Playing);
            if player.playback == Playback::Playing && !was_playing {
                self.promote(&player.bus_name);
            } else if !self.recent.contains(&player.bus_name) {
                self.recent.push(player.bus_name.clone());
            }
        }
        self.recent
            .retain(|name| fresh.iter().any(|player| player.bus_name == *name));
        self.players = fresh;
    }

    pub fn promote(&mut self, bus_name: &str) {
        self.recent.retain(|name| name != bus_name);
        self.recent.insert(0, bus_name.to_owned());
    }

    pub fn current(&self) -> Option<&Player> {
        let first = self.recent.first()?;
        self.players.iter().find(|player| player.bus_name == *first)
    }

    pub fn all(&self) -> impl Iterator<Item = &Player> {
        self.recent
            .iter()
            .filter_map(|name| self.players.iter().find(|player| player.bus_name == *name))
    }

    pub fn len(&self) -> usize {
        self.players.len()
    }

    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }
}

fn properties(
    connection: &Connection,
    bus_name: &str,
    interface: &str,
) -> zbus::Result<Properties> {
    let proxy = PropertiesProxy::builder(connection)
        .destination(bus_name)?
        .path(OBJECT_PATH)?
        .build()?;
    Ok(proxy.get_all(InterfaceName::try_from(interface)?)?)
}

fn fetch(connection: &Connection, bus_name: &str) -> Option<Player> {
    let root = properties(connection, bus_name, ROOT_INTERFACE).unwrap_or_default();
    let player = properties(connection, bus_name, PLAYER_INTERFACE)
        .inspect_err(|error| tracing::debug!("{bus_name} does not answer: {error}"))
        .ok()?;
    Some(Player::read(bus_name, &root, &player, Instant::now()))
}

pub fn players(connection: &Connection) -> Vec<Player> {
    DBusProxy::new(connection)
        .and_then(|bus| Ok(bus.list_names()?))
        .unwrap_or_default()
        .iter()
        .filter(|name| name.as_str().starts_with(PLAYER_PREFIX))
        .filter_map(|name| fetch(connection, name.as_str()))
        .collect()
}

fn concerns_players(message: &message::Message) -> bool {
    let header = message.header();
    if header
        .path()
        .is_some_and(|path| path.as_str() == OBJECT_PATH)
    {
        return true;
    }
    header
        .member()
        .is_some_and(|member| member == "NameOwnerChanged")
        && message
            .body()
            .deserialize::<(String, String, String)>()
            .is_ok_and(|(name, _, _)| name.starts_with(PLAYER_PREFIX))
}

pub fn watch(connection: &Connection, outbox: &Outbox<Vec<Player>>) -> zbus::Result<()> {
    let rule = MatchRule::builder().msg_type(message::Type::Signal).build();
    let signals = MessageIterator::for_match_rule(rule, connection, Some(256))?;
    let mut shown = players(connection);
    if !outbox.send(shown.clone()) {
        return Ok(());
    }
    for signal in signals.flatten() {
        if !concerns_players(&signal) {
            continue;
        }
        let current = players(connection);
        let changed = current.len() != shown.len()
            || current
                .iter()
                .zip(&shown)
                .any(|(now, before)| !now.same_as(before));
        let seeked = signal
            .header()
            .member()
            .is_some_and(|member| member == "Seeked");
        if changed || seeked {
            shown = current;
            if !outbox.send(shown.clone()) {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    PlayPause,
    Pause,
    Next,
    Previous,
    Raise,
    SetPosition {
        track: Option<String>,
        position: Duration,
        from: Duration,
    },
}

fn signed_micros(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

pub fn send(connection: &Connection, bus_name: &str, command: &Command) -> zbus::Result<()> {
    let call = |interface: &str, method: &str| {
        connection
            .call_method(Some(bus_name), OBJECT_PATH, Some(interface), method, &())
            .map(|_| ())
    };
    match command {
        Command::PlayPause => call(PLAYER_INTERFACE, "PlayPause"),
        Command::Pause => call(PLAYER_INTERFACE, "Pause"),
        Command::Next => call(PLAYER_INTERFACE, "Next"),
        Command::Previous => call(PLAYER_INTERFACE, "Previous"),
        Command::Raise => call(ROOT_INTERFACE, "Raise"),
        Command::SetPosition {
            track: Some(track),
            position,
            ..
        } => connection
            .call_method(
                Some(bus_name),
                OBJECT_PATH,
                Some(PLAYER_INTERFACE),
                "SetPosition",
                &(
                    ObjectPath::try_from(track.as_str())?,
                    signed_micros(*position),
                ),
            )
            .map(|_| ()),
        Command::SetPosition {
            track: None,
            position,
            from,
        } => connection
            .call_method(
                Some(bus_name),
                OBJECT_PATH,
                Some(PLAYER_INTERFACE),
                "Seek",
                &(signed_micros(*position) - signed_micros(*from)),
            )
            .map(|_| ()),
    }
}

pub fn run_commands(connection: &Connection, commands: &Receiver<(String, Command)>) {
    for (bus_name, command) in commands {
        if let Err(error) = send(connection, &bus_name, &command) {
            tracing::warn!("{bus_name} did not take {command:?}: {error}");
        }
    }
}
