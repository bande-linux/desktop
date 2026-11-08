use std::sync::OnceLock;
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::thread;
use std::time::SystemTime;

use zbus::blocking::Connection;
use zbus::blocking::object_server::InterfaceRef;

use crate::state::Compositor;

const NAME: &str = "org.bande.Displays";
const PATH: &str = "/org/bande/Displays";

pub type ConnectedDisplay = (String, String, String);

struct Displays {
    connected: Arc<Mutex<Vec<ConnectedDisplay>>>,
}

#[zbus::interface(name = "org.bande.Displays")]
impl Displays {
    #[zbus(property)]
    fn connected(&self) -> Vec<ConnectedDisplay> {
        self.connected
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub struct DisplaysHandle {
    connected: Arc<Mutex<Vec<ConnectedDisplay>>>,
    changed: mpsc::Sender<()>,
}

impl DisplaysHandle {
    pub fn publish(&self, displays: Vec<ConnectedDisplay>) {
        let mut connected = self
            .connected
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *connected != displays {
            *connected = displays;
            let _ = self.changed.send(());
        }
    }
}

fn announce(interface: InterfaceRef<Displays>, changed: mpsc::Receiver<()>) {
    for () in changed {
        let emitted = zbus::block_on(
            interface
                .get()
                .connected_changed(interface.signal_emitter()),
        );
        if let Err(error) = emitted {
            tracing::warn!("cannot announce the connected displays: {error}");
        }
    }
}

pub fn serve_displays(connection: &Connection) -> zbus::Result<DisplaysHandle> {
    let connected = Arc::default();
    let server = connection.object_server();
    server.at(
        PATH,
        Displays {
            connected: Arc::clone(&connected),
        },
    )?;
    let interface = server.interface::<_, Displays>(PATH)?;
    connection.request_name(NAME)?;
    let (changed, received) = mpsc::channel();
    thread::spawn(move || announce(interface, received));
    Ok(DisplaysHandle { connected, changed })
}

impl Compositor {
    pub fn session_facts(&self) -> String {
        static STARTED: OnceLock<u64> = OnceLock::new();
        let started = STARTED.get_or_init(|| {
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs())
        });
        let displays: Vec<String> = self
            .state
            .screens
            .iter()
            .filter(|screen| !screen.parked)
            .map(|screen| {
                let mode = screen
                    .output
                    .current_mode()
                    .map_or_else(String::new, |mode| {
                        format!("{}x{} {}mHz", mode.size.w, mode.size.h, mode.refresh)
                    });
                format!("\"{} {mode} scale {}\"", screen.config.name, screen.scale())
            })
            .collect();
        format!(
            r#"{{"started":{started},"style":"{}","displays":[{}]}}"#,
            self.style_keyword(),
            displays.join(",")
        )
    }

    pub fn connected_displays(&self) -> Vec<ConnectedDisplay> {
        self.state
            .screens
            .iter()
            .filter(|screen| !screen.parked)
            .map(|screen| {
                let config = &screen.config;
                (
                    config.name.clone(),
                    config.identity(),
                    bande_edid::display_name(&config.make, &config.model)
                        .unwrap_or_else(|| config.name.clone()),
                )
            })
            .collect()
    }
}
