use std::net::IpAddr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::Duration;

use bande_config::Config;
use bande_keyring::Keyring;
use bande_l10n::tr;
use bande_notify::Urgency;
use bande_notify::client::{self, Listener, Request, Signal};
use bande_screen_sharing::rfb::{Host, Viewing};
use bande_screen_sharing::setup::{KEYRING_APPLICATION, PASSWORD_KEY};
use zbus::blocking::Connection;

use crate::capture::Capture;

const ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
const ALLOW: &str = "allow";
const DENY: &str = "deny";
const SETTINGS: &str = "org.bande.Settings";

pub struct Desktop {
    session: Connection,
    password: Option<String>,
    capture: Mutex<Weak<Capture>>,
    name: String,
}

fn config() -> Config {
    let Some(path) = bande_config::config_path() else {
        return Config::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => Config::load(&text).map_or_else(
            |error| {
                tracing::warn!("cannot read the settings: {error}");
                Config::default()
            },
            |(config, _)| config,
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(error) => {
            tracing::warn!("cannot read the settings: {error}");
            Config::default()
        }
    }
}

impl Desktop {
    pub fn new(session: Connection, name: String) -> Self {
        let password = Keyring::open(KEYRING_APPLICATION)
            .and_then(|keyring| keyring.password(&PASSWORD_KEY))
            .inspect_err(|error| tracing::warn!("cannot read the Screen Sharing password: {error}"))
            .ok()
            .flatten()
            .filter(|password| !password.is_empty());
        Self {
            session,
            password,
            capture: Mutex::new(Weak::new()),
            name,
        }
    }

    fn capture(&self) -> MutexGuard<'_, Weak<Capture>> {
        self.capture.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Disconnects every viewer when the screenshot tool stopped showing the screen.
    pub fn ended(&self, nodes: &[u32]) {
        if let Some(capture) = self.capture().upgrade()
            && nodes.contains(&capture.node)
        {
            capture.screen.end();
        }
    }

    fn notify(&self, summary: &str, body: &str, actions: &[(&str, &str)]) -> zbus::Result<u32> {
        client::notify(
            &self.session,
            &Request {
                app_name: &tr!("Screen Sharing"),
                replaces_id: 0,
                app_icon: SETTINGS,
                summary,
                body,
                actions,
                urgency: match actions.is_empty() {
                    true => Urgency::Normal,
                    false => Urgency::Critical,
                },
                sound: None,
                desktop_entry: Some(SETTINGS),
            },
        )
    }

    fn ask(&self, peer: IpAddr) -> zbus::Result<bool> {
        let listener = Listener::new(&self.session)?;
        let (answer, answered) = mpsc::channel();
        let id = self.notify(
            &tr!("Let someone see and control this screen?"),
            &tr!(
                "A viewer at {address} knows the Screen Sharing password.",
                address = peer.to_string()
            ),
            &[(DENY, &tr!("Deny")), (ALLOW, &tr!("Allow"))],
        )?;
        thread::spawn(move || {
            listener.listen(|signal| match signal {
                Signal::ActionInvoked(shown, key) if shown == id => {
                    let _ = answer.send(key == ALLOW);
                    false
                }
                Signal::Closed(shown) if shown == id => {
                    let _ = answer.send(false);
                    false
                }
                _ => true,
            });
        });
        let allowed = answered.recv_timeout(ANSWER_TIMEOUT).unwrap_or(false);
        if let Err(error) = client::close(&self.session, id) {
            tracing::debug!("cannot close the question: {error}");
        }
        Ok(allowed)
    }
}

impl Host for Desktop {
    fn password(&self) -> Option<String> {
        self.password.clone()
    }

    fn older_viewers(&self) -> bool {
        config().screen_sharing_older_viewers
    }

    fn admit(&self, peer: IpAddr) -> bool {
        if config().screen_sharing_ask {
            return self
                .ask(peer)
                .inspect_err(|error| tracing::warn!("cannot ask about a viewer: {error}"))
                .unwrap_or(false);
        }
        if let Err(error) = self.notify(
            &tr!("Someone is viewing this screen"),
            &tr!(
                "A viewer at {address} can see and control it. Click the screen symbol in the Taskbar to stop.",
                address = peer.to_string()
            ),
            &[],
        ) {
            tracing::warn!("cannot say that someone is viewing the screen: {error}");
        }
        true
    }

    fn view(&self, peer: IpAddr) -> Option<Viewing> {
        let mut kept = self.capture();
        let capture = match kept.upgrade().filter(|capture| !capture.screen.is_ended()) {
            Some(capture) => capture,
            None => {
                let capture = Capture::start(&self.session)
                    .inspect_err(|error| {
                        tracing::warn!("cannot show the screen to {peer}: {error}");
                    })
                    .ok()?;
                let capture = Arc::new(capture);
                *kept = Arc::downgrade(&capture);
                capture
            }
        };
        Some(Viewing {
            screen: Arc::clone(&capture.screen),
            input: Arc::clone(&capture.input) as _,
            kept: capture,
        })
    }

    fn name(&self) -> String {
        self.name.clone()
    }
}
