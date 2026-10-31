#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bande_controlcenter::media::{self, Command, Playback, Player, Players};
use bande_inbox::Inbox;
use bande_test_bus::SessionBus;
use zbus::blocking::Connection;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

#[derive(Default)]
struct Calls(Mutex<Vec<String>>);

struct Root;

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    #[zbus(property)]
    fn identity(&self) -> String {
        "Tunes".to_owned()
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "org.example.Tunes".to_owned()
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }
}

struct Transport {
    calls: Arc<Calls>,
    status: String,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Transport {
    fn play_pause(&self) {
        self.calls.0.lock().unwrap().push("PlayPause".to_owned());
    }

    fn next(&self) {
        self.calls.0.lock().unwrap().push("Next".to_owned());
    }

    fn set_position(&self, track: ObjectPath<'_>, position: i64) {
        self.calls
            .0
            .lock()
            .unwrap()
            .push(format!("SetPosition {track} {position}"));
    }

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.status.clone()
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let value = |value: Value<'_>| value.try_to_owned().unwrap();
        HashMap::from([
            (
                "xesam:title".to_owned(),
                value(Value::from("Harbour Lights")),
            ),
            (
                "xesam:artist".to_owned(),
                value(Value::from(vec!["Ada", "Grace"])),
            ),
            (
                "mpris:length".to_owned(),
                value(Value::from(200_000_000_i64)),
            ),
            (
                "mpris:artUrl".to_owned(),
                value(Value::from("file:///tmp/cover.png")),
            ),
            (
                "mpris:trackid".to_owned(),
                value(Value::from(
                    ObjectPath::try_from("/org/example/track/7").unwrap(),
                )),
            ),
        ])
    }

    #[zbus(property)]
    fn position(&self) -> i64 {
        30_000_000
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        true
    }
}

fn serve(bus: &SessionBus, name: &str, status: &str) -> (Connection, Arc<Calls>) {
    let calls = Arc::new(Calls::default());
    let connection = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name(name)
        .expect("name")
        .serve_at("/org/mpris/MediaPlayer2", Root)
        .expect("root")
        .serve_at(
            "/org/mpris/MediaPlayer2",
            Transport {
                calls: Arc::clone(&calls),
                status: status.to_owned(),
            },
        )
        .expect("player")
        .build()
        .expect("player connection");
    (connection, calls)
}

#[test]
fn a_player_on_the_bus_is_read_with_its_track_and_abilities() {
    let bus = SessionBus::start().expect("bus");
    let _player = serve(&bus, "org.mpris.MediaPlayer2.tunes", "Playing");
    let client = bus.connect().expect("client");

    let players = media::players(&client);

    assert_eq!(players.len(), 1);
    let player = &players[0];
    assert_eq!(player.bus_name, "org.mpris.MediaPlayer2.tunes");
    assert_eq!(player.identity, "Tunes");
    assert_eq!(player.desktop_entry.as_deref(), Some("org.example.Tunes"));
    assert_eq!(player.title, "Harbour Lights");
    assert_eq!(player.artist, "Ada, Grace");
    assert_eq!(player.art_url.as_deref(), Some("file:///tmp/cover.png"));
    assert_eq!(player.track.as_deref(), Some("/org/example/track/7"));
    assert_eq!(player.playback, Playback::Playing);
    assert_eq!(player.length, Some(Duration::from_secs(200)));
    assert_eq!(player.position, Some(Duration::from_secs(30)));
    assert!(player.can_go_next && player.can_seek && player.can_raise);
    assert!(!player.can_go_previous);
}

#[test]
fn commands_reach_the_player_and_seeking_names_the_track() {
    let bus = SessionBus::start().expect("bus");
    let (_player, calls) = serve(&bus, "org.mpris.MediaPlayer2.tunes", "Paused");
    let client = bus.connect().expect("client");
    let name = "org.mpris.MediaPlayer2.tunes";

    media::send(&client, name, &Command::PlayPause).expect("play");
    media::send(&client, name, &Command::Next).expect("next");
    media::send(
        &client,
        name,
        &Command::SetPosition {
            track: Some("/org/example/track/7".to_owned()),
            position: Duration::from_secs(90),
            from: Duration::from_secs(30),
        },
    )
    .expect("seek");

    assert_eq!(
        *calls.0.lock().unwrap(),
        [
            "PlayPause",
            "Next",
            "SetPosition /org/example/track/7 90000000"
        ]
    );
}

#[test]
fn a_change_in_a_player_is_reported_to_control_center() {
    let bus = SessionBus::start().expect("bus");
    let client = bus.connect().expect("client");
    let mut inbox = Inbox::new().expect("inbox");
    let outbox = inbox.outbox();
    let watching = client.clone();
    std::thread::spawn(move || media::watch(&watching, &outbox));
    let received = |inbox: &mut Inbox<Vec<Player>>, ready: &dyn Fn(&[Player]) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if inbox.drain().iter().any(|players| ready(players)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    };
    assert!(received(&mut inbox, &|players| players.is_empty()));

    let (player, _) = serve(&bus, "org.mpris.MediaPlayer2.tunes", "Paused");
    assert!(received(&mut inbox, &|players| {
        players.first().map(|player| player.playback) == Some(Playback::Paused)
    }));

    let transport = player
        .object_server()
        .interface::<_, Transport>("/org/mpris/MediaPlayer2")
        .expect("player interface");
    transport.get_mut().status = "Playing".to_owned();
    zbus::block_on(
        transport
            .get()
            .playback_status_changed(transport.signal_emitter()),
    )
    .expect("signal");
    assert!(received(&mut inbox, &|players| {
        players.first().map(|player| player.playback) == Some(Playback::Playing)
    }));

    drop((transport, player));
    assert!(received(&mut inbox, &|players| players.is_empty()));
}

fn player(name: &str, playback: Playback) -> Player {
    Player {
        bus_name: name.to_owned(),
        identity: name.to_owned(),
        desktop_entry: None,
        title: String::new(),
        artist: String::new(),
        art_url: None,
        track: None,
        playback,
        can_play: true,
        can_pause: true,
        can_go_next: true,
        can_go_previous: true,
        can_seek: true,
        can_raise: false,
        position: Some(Duration::from_secs(10)),
        length: Some(Duration::from_secs(60)),
        rate: 1.0,
        sampled: Instant::now(),
    }
}

fn order(players: &Players) -> Vec<&str> {
    players
        .all()
        .map(|player| player.bus_name.as_str())
        .collect()
}

#[test]
fn the_player_that_started_playing_last_is_the_one_shown_and_controlled() {
    let mut players = Players::default();
    players.update(vec![
        player("radio", Playback::Paused),
        player("video", Playback::Stopped),
    ]);
    assert_eq!(order(&players), ["radio", "video"]);

    players.update(vec![
        player("radio", Playback::Paused),
        player("video", Playback::Playing),
    ]);
    assert_eq!(
        players.current().map(|found| found.bus_name.as_str()),
        Some("video")
    );

    players.update(vec![
        player("radio", Playback::Playing),
        player("video", Playback::Playing),
    ]);
    assert_eq!(order(&players), ["radio", "video"]);

    players.update(vec![
        player("radio", Playback::Playing),
        player("video", Playback::Playing),
    ]);
    assert_eq!(
        order(&players),
        ["radio", "video"],
        "still playing is no news"
    );

    players.promote("video");
    assert_eq!(order(&players), ["video", "radio"], "the user's pick wins");

    players.update(vec![player("radio", Playback::Paused)]);
    assert_eq!(order(&players), ["radio"]);
}

#[test]
fn the_position_moves_on_while_playing_and_stops_at_the_end() {
    let mut playing = player("radio", Playback::Playing);
    let start = playing.sampled;
    assert_eq!(
        playing.position_at(start + Duration::from_secs(5)),
        Some(Duration::from_secs(15))
    );
    assert_eq!(
        playing.position_at(start + Duration::from_secs(500)),
        Some(Duration::from_secs(60))
    );
    playing.rate = 2.0;
    assert_eq!(
        playing.position_at(start + Duration::from_secs(5)),
        Some(Duration::from_secs(20))
    );
    let paused = player("radio", Playback::Paused);
    assert_eq!(
        paused.position_at(paused.sampled + Duration::from_secs(5)),
        Some(Duration::from_secs(10))
    );
}
