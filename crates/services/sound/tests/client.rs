#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use unbound_sound::{BUS_NAME, OBJECT_PATH, play};
use unbound_test_bus::SessionBus;

struct Player(Mutex<Sender<String>>);

#[zbus::interface(name = "org.unbound.Sound")]
impl Player {
    fn play(&self, name: &str) {
        let _ = self.0.lock().unwrap().send(name.to_owned());
    }
}

#[test]
fn a_shell_component_asks_the_sound_service_to_play_without_waiting_for_it() {
    let bus = SessionBus::start().expect("bus");
    let (sender, played) = mpsc::channel();
    let _service = zbus::blocking::connection::Builder::address(bus.address())
        .expect("address")
        .name(BUS_NAME)
        .expect("name")
        .serve_at(OBJECT_PATH, Player(Mutex::new(sender)))
        .expect("serve")
        .build()
        .expect("service");
    let shell = bus.connect().expect("shell");

    play(&shell, "screen-capture");
    play(&shell, "trash-empty");

    let timeout = Duration::from_secs(5);
    assert_eq!(
        played.recv_timeout(timeout).as_deref(),
        Ok("screen-capture")
    );
    assert_eq!(played.recv_timeout(timeout).as_deref(), Ok("trash-empty"));
}

#[test]
fn playing_without_a_sound_service_is_harmless() {
    let bus = SessionBus::start().expect("bus");
    let shell = bus.connect().expect("shell");
    play(&shell, "bell");
}
