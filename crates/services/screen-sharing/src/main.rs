mod capture;
mod host;
mod input;
mod network;

use std::error::Error;
use std::net::{Ipv4Addr, Ipv6Addr, TcpListener};
use std::process::ExitCode;
use std::sync::{Arc, Weak};
use std::thread;

use bande_screen_sharing::rfb::identity::Identity;
use bande_screen_sharing::rfb::serve;
use bande_screen_sharing::setup::PORT;
use zbus::blocking::{Connection, MessageIterator};
use zbus::{MatchRule, message};

use crate::capture::{SCREENSHOT_INTERFACE, SCREENSHOT_PATH};
use crate::host::Desktop;

const HOSTNAME: &str = "/proc/sys/kernel/hostname";
const SERVICE_KIND: &str = "_rfb._tcp";

fn computer_name() -> String {
    std::fs::read_to_string(HOSTNAME)
        .map(|name| name.trim().to_owned())
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Bande".to_owned())
}

fn listen() -> std::io::Result<TcpListener> {
    TcpListener::bind((Ipv6Addr::UNSPECIFIED, PORT)).or_else(|error| {
        tracing::info!("listening over IPv4 only: {error}");
        TcpListener::bind((Ipv4Addr::UNSPECIFIED, PORT))
    })
}

fn follow_ended(session: &Connection, desktop: Weak<Desktop>) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(SCREENSHOT_INTERFACE)?
        .path(SCREENSHOT_PATH)?
        .member("StreamsEnded")?
        .build();
    let signals = MessageIterator::for_match_rule(rule, session, None)?;
    thread::Builder::new()
        .name("ended".into())
        .spawn(move || {
            for signal in signals.flatten() {
                let Some(desktop) = desktop.upgrade() else {
                    return;
                };
                if let Ok((nodes,)) = signal.body().deserialize::<(Vec<u32>,)>() {
                    desktop.ended(&nodes);
                }
            }
        })
        .map_err(|error| zbus::Error::Failure(error.to_string()))?;
    Ok(())
}

fn run() -> Result<(), Box<dyn Error>> {
    let session = bande_bus::session()?;
    let name = computer_name();
    let tls = Identity::kept(&name)?.tls()?;
    let listener = listen()?;
    let desktop = Arc::new(Desktop::new(session.clone(), name.clone()));
    follow_ended(&session, Arc::downgrade(&desktop))?;
    let _announced = match bande_discovery::nearby::announce(&name, SERVICE_KIND, PORT) {
        Ok(Some(announced)) => Some(announced),
        Ok(None) => {
            tracing::info!("Avahi is not installed, so viewers must type this computer's address");
            None
        }
        Err(error) => {
            tracing::warn!("cannot announce Screen Sharing on the network: {error}");
            None
        }
    };
    tracing::info!("viewers on the local network can connect on port {PORT}");
    serve(&listener, desktop, tls, network::on_local_network)?;
    Ok(())
}

fn main() -> ExitCode {
    bande_log::init("bande-screen-sharing");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
