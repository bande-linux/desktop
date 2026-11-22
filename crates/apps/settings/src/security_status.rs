use std::sync::mpsc::{self, Sender};

use bande_l10n::{text, tr};
use bande_packages::Client;
use bande_settings_panes::PaneId;
use bande_toolkit::ui::Toggle;

use crate::failure::pane_failed;
use crate::follow::{Update, Updates, spawn};
use crate::pages::{Widgets, boxed};
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use zbus::blocking::{Connection, Proxy};

const REMOTE_LOGIN_UNIT: &str = "ssh.service";
const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const CONNECTIVITY_CHECK: &str = "network-manager-config-connectivity-debian";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecurityStatus {
    pub remote_login: bool,
    pub sign_in_pages: Option<SignInPages>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignInPages {
    pub on: bool,
    pub server: Option<String>,
}

fn read_remote_login(system: &Connection) -> bool {
    let state: zbus::Result<String> = system
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "GetUnitFileState",
            &(REMOTE_LOGIN_UNIT,),
        )
        .and_then(|reply| reply.body().deserialize());
    state.is_ok_and(|state| state == "enabled")
}

fn network_manager(system: &Connection) -> zbus::Result<Proxy<'static>> {
    Proxy::new(
        system,
        NETWORK_MANAGER,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER,
    )
}

fn server_of(uri: &str) -> Option<String> {
    let rest = uri.split_once("://").map_or(uri, |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| host.to_owned())
}

fn read_sign_in_pages(system: &Connection) -> zbus::Result<SignInPages> {
    let manager = network_manager(system)?;
    let available: bool = manager.get_property("ConnectivityCheckAvailable")?;
    let enabled: bool = manager.get_property("ConnectivityCheckEnabled")?;
    let uri: String = manager.get_property("ConnectivityCheckUri")?;
    Ok(SignInPages {
        on: available && enabled,
        server: available.then(|| server_of(&uri)).flatten(),
    })
}

fn read(system: Option<&Connection>) -> SecurityStatus {
    let Some(system) = system else {
        return SecurityStatus::default();
    };
    SecurityStatus {
        remote_login: read_remote_login(system),
        sign_in_pages: read_sign_in_pages(system)
            .inspect_err(|error| tracing::info!("no NetworkManager connectivity check: {error}"))
            .ok(),
    }
}

fn detect_sign_in_pages(system: &Connection, on: bool) -> Result<(), String> {
    let manager = network_manager(system).map_err(|error| error.to_string())?;
    let available: bool = manager
        .get_property("ConnectivityCheckAvailable")
        .map_err(|error| error.to_string())?;
    if on && !available {
        Client::new(system)
            .install_missing(&[CONNECTIVITY_CHECK])
            .map_err(|error| error.to_string())?;
    }
    manager
        .set_property("ConnectivityCheckEnabled", on)
        .map_err(|error| error.to_string())
}

pub fn follow_security_status(system: Option<&Connection>, updates: &Updates) -> Sender<bool> {
    let (sender, receiver) = mpsc::channel();
    let system = system.cloned();
    let updates = updates.clone();
    spawn("security", move || {
        updates.send(Update::Security(read(system.as_ref())));
        for on in receiver {
            if let Some(system) = &system
                && let Err(error) = detect_sign_in_pages(system, on)
            {
                updates.send(pane_failed(
                    PaneId::PrivacySecurity,
                    text!("Couldn't change sign-in page detection."),
                    error,
                ));
            }
            if !updates.send(Update::Security(read(system.as_ref()))) {
                return;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn security_status_rows(&mut self) -> Widgets {
        let status = self.security.clone();
        let remote_detail = if status.remote_login {
            tr!("On. Others on your network can try to sign in over SSH.")
        } else {
            tr!("Off.")
        };
        let mut rows = vec![
            boxed(
                Row::new(Text::new(&tr!("Remote Login"), Tone::Primary), None)
                    .detail(Text::new(&remote_detail, Tone::Muted).caption()),
            ),
            boxed(
                Row::new(Text::new(&tr!("Apps you install"), Tone::Primary), None)
                    .detail(Text::new(&tr!("Apps from Flatpak are sandboxed. Other programs run with your full access to your files, and nothing here can contain them, so only run programs you trust."), Tone::Muted).caption()),
            ),
        ];
        if let Some(sign_in_pages) = status.sign_in_pages {
            let label = tr!("Detect sign-in pages on networks");
            let change = intent(&self.intents, Intent::SignInPages);
            let toggle = Toggle::new(&label, sign_in_pages.on, change);
            let detail = match sign_in_pages.server {
                Some(server) => tr!(
                    "Contacts {server} whenever a network connects, to find out whether it asks you to sign in",
                    server = server
                ),
                None => tr!(
                    "Contacts Debian’s server whenever a network connects, to find out whether it asks you to sign in"
                ),
            };
            let row = Row::new(Text::new(&label, Tone::Primary), Some(boxed(toggle)))
                .detail(Text::new(&detail, Tone::Muted).caption());
            rows.push(self.item_row("sign-in-pages".to_owned(), row));
        }
        rows
    }
}
