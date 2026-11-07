use std::sync::{Arc, Mutex, PoisonError};

use smithay::reexports::calloop::channel::Sender;
use zbus::blocking::Connection;
use zbus::blocking::object_server::InterfaceRef;

use crate::state::Compositor;

const NAME: &str = "org.bande.Displays";
const PATH: &str = "/org/bande/Displays";

pub type ConnectedDisplay = (String, String, String, bool);

struct Displays {
    connected: Arc<Mutex<Vec<ConnectedDisplay>>>,
    dims: Sender<(String, f64)>,
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

    fn set_dim(&self, connector: String, gain: f64) -> zbus::fdo::Result<()> {
        self.dims
            .send((connector, gain.clamp(0.0, 1.0)))
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}

pub struct DisplaysHandle {
    connected: Arc<Mutex<Vec<ConnectedDisplay>>>,
    interface: InterfaceRef<Displays>,
}

impl DisplaysHandle {
    pub fn publish(&self, displays: Vec<ConnectedDisplay>) {
        {
            let mut connected = self
                .connected
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if *connected == displays {
                return;
            }
            *connected = displays;
        }
        let emitted = zbus::block_on(
            self.interface
                .get()
                .connected_changed(self.interface.signal_emitter()),
        );
        if let Err(error) = emitted {
            tracing::warn!("cannot announce the connected displays: {error}");
        }
    }
}

pub fn serve_displays(
    connection: &Connection,
    dims: Sender<(String, f64)>,
) -> zbus::Result<DisplaysHandle> {
    let connected = Arc::default();
    let server = connection.object_server();
    server.at(
        PATH,
        Displays {
            connected: Arc::clone(&connected),
            dims,
        },
    )?;
    let interface = server.interface::<_, Displays>(PATH)?;
    connection.request_name(NAME)?;
    Ok(DisplaysHandle {
        connected,
        interface,
    })
}

impl Compositor {
    pub fn connected_displays(
        &self,
        can_dim: &mut impl FnMut(&str) -> bool,
    ) -> Vec<ConnectedDisplay> {
        self.state
            .screens
            .iter()
            .map(|screen| {
                let config = &screen.config;
                (
                    config.name.clone(),
                    config.identity(),
                    bande_edid::display_name(&config.make, &config.model)
                        .unwrap_or_else(|| config.name.clone()),
                    can_dim(&config.name),
                )
            })
            .collect()
    }
}
