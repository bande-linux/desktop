#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bande_power::brightness::{BrightnessProxyBlocking, Lights, Options};
use bande_power::ddc::Wire;
use bande_test_bus::SessionBus;
use zbus::blocking::object_server::InterfaceRef;

type Connected = (String, String, String, bool);

#[derive(Default)]
struct Screens {
    connected: Vec<Connected>,
    dims: Vec<(String, f64)>,
}

struct Compositor {
    screens: Arc<Mutex<Screens>>,
}

#[zbus::interface(name = "org.bande.Displays")]
impl Compositor {
    #[zbus(property)]
    fn connected(&self) -> Vec<Connected> {
        self.screens.lock().unwrap().connected.clone()
    }

    fn set_dim(&self, connector: String, gain: f64) {
        self.screens.lock().unwrap().dims.push((connector, gain));
    }
}

struct Panel {
    value: u16,
    writes: usize,
    answering: bool,
    reply: Vec<u8>,
}

struct PanelWire(Arc<Mutex<Panel>>);

impl Wire for PanelWire {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut panel = self.0.lock().unwrap();
        if !panel.answering {
            return Err(io::Error::from_raw_os_error(6));
        }
        assert_eq!(bytes.iter().fold(0x6E, |sum, byte| sum ^ byte), 0);
        match bytes[2] {
            0x01 => {
                let [high, low] = panel.value.to_be_bytes();
                let mut reply = vec![0x6E, 0x88, 0x02, 0, 0x10, 0, 0, 100, high, low];
                reply.push(reply.iter().fold(0x50, |sum, byte| sum ^ byte));
                panel.reply = reply;
            }
            0x03 => {
                panel.value = u16::from_be_bytes([bytes[4], bytes[5]]);
                panel.writes += 1;
            }
            other => panic!("unexpected DDC opcode {other}"),
        }
        Ok(())
    }

    fn receive(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        bytes.copy_from_slice(&self.0.lock().unwrap().reply);
        Ok(())
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < Duration::from_secs(10), "{what}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn head(connector: &str) -> Connected {
    let name = format!("Maker {connector}");
    (connector.to_owned(), name.clone(), name, true)
}

#[test]
fn every_display_gets_a_bar_and_ddc_writes_are_coalesced() {
    let bus = SessionBus::start().expect("dbus-daemon");
    let shell = bus.connect().expect("compositor connection");
    let screens = Arc::new(Mutex::new(Screens {
        connected: vec![head("DP-1"), head("HDMI-A-1")],
        dims: Vec::new(),
    }));
    let server = shell.object_server();
    server
        .at(
            "/org/bande/Displays",
            Compositor {
                screens: Arc::clone(&screens),
            },
        )
        .expect("serve the compositor");
    let compositor: InterfaceRef<Compositor> = server
        .interface("/org/bande/Displays")
        .expect("the compositor interface");
    shell.request_name("org.bande.Displays").expect("the name");

    let panel = Arc::new(Mutex::new(Panel {
        value: 50,
        writes: 0,
        answering: true,
        reply: Vec::new(),
    }));
    let state = tempfile::tempdir().expect("state folder");
    let wired = Arc::clone(&panel);
    let session = bus.connect().expect("service connection");
    let _lights = Lights::start(
        &session,
        None,
        Options {
            backlights: Path::new("/nonexistent").to_owned(),
            saved: Some(state.path().join("brightness")),
            open_wire: Arc::new(move |connector| {
                (connector == "DP-1")
                    .then(|| Box::new(PanelWire(Arc::clone(&wired))) as Box<dyn Wire>)
            }),
        },
    )
    .expect("lights");

    let client = bus.connect().expect("client connection");
    let brightness = BrightnessProxyBlocking::builder(&client)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .expect("proxy");
    let level = |connector: &str| {
        brightness
            .displays()
            .expect("displays")
            .into_iter()
            .find(|(known, _, _)| known == connector)
            .map(|(_, _, level)| level)
    };

    wait_until("the monitor reports its own level", || {
        level("DP-1") == Some(0.5)
    });
    assert_eq!(level("HDMI-A-1"), Some(1.0));

    for step in 0..=100 {
        brightness
            .set_level("DP-1", 0.2 + f64::from(step) * 0.005)
            .expect("set the level");
        thread::sleep(Duration::from_millis(5));
    }
    wait_until("the last level reaches the monitor", || {
        panel.lock().unwrap().value == 70
    });
    assert!(panel.lock().unwrap().writes <= 6);

    let dimmed = |connector: &str| {
        screens
            .lock()
            .unwrap()
            .dims
            .iter()
            .any(|(dimmed, gain)| dimmed == connector && *gain < 1.0)
    };
    brightness.set_level("HDMI-A-1", 0.0).expect("set");
    wait_until("the display without DDC is dimmed", || dimmed("HDMI-A-1"));

    let replug = |connected: Vec<Connected>| {
        screens.lock().unwrap().connected = connected;
        zbus::block_on(
            compositor
                .get()
                .connected_changed(compositor.signal_emitter()),
        )
        .expect("announce");
    };
    replug(vec![head("DP-1")]);
    wait_until("the unplugged display loses its bar", || {
        level("HDMI-A-1").is_none()
    });
    replug(vec![head("DP-1"), head("HDMI-A-1")]);
    wait_until("the display comes back at its level", || {
        level("HDMI-A-1") == Some(0.0)
    });

    panel.lock().unwrap().answering = false;
    brightness.set_level("DP-1", 0.3).expect("set");
    wait_until("a monitor that stops answering is dimmed", || {
        dimmed("DP-1")
    });
    assert_eq!(level("DP-1"), Some(0.3));
}
