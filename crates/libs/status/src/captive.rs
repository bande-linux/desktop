use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_l10n::tr;
use bande_network::{JoinOutcome, Network, NetworkManager, Security};
use bande_notify::client::{self, Request};
use bande_notify::{DEFAULT_ACTION, Urgency};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const CONNECTIVITY: &str = "Connectivity";
const OFFER_ACTION: &str = "sign-in";
const NETWORK_ICON: &str = "network-wireless-symbolic";

static OFFERED: Mutex<Option<u32>> = Mutex::new(None);

fn lock(shown: &Mutex<Option<u32>>) -> MutexGuard<'_, Option<u32>> {
    shown.lock().unwrap_or_else(PoisonError::into_inner)
}

fn sign_in_summary(network: Option<&str>) -> String {
    match network {
        Some(network) => tr!("Sign in to {network}", network = network.to_owned()),
        None => tr!("Sign in to the network"),
    }
}

fn tell(session: &Connection, summary: &str, body: &str) -> zbus::Result<u32> {
    client::notify(
        session,
        &Request {
            app_name: &tr!("Network"),
            replaces_id: 0,
            app_icon: NETWORK_ICON,
            summary,
            body,
            actions: &[],
            urgency: Urgency::Normal,
            sound: None,
            desktop_entry: None,
        },
    )
}

pub fn join_network(network: &NetworkManager, found: &Network) {
    let reason = match network.join(found, None) {
        Ok(JoinOutcome::Joined) => return offer_sign_in(network, found),
        Ok(outcome) => outcome.to_string(),
        Err(error) => error.to_string(),
    };
    tracing::warn!("cannot join {}: {reason}", found.name());
    let summary = tr!("Could not join {network}", network = found.name());
    if let Err(error) = Connection::session().and_then(|session| tell(&session, &summary, &reason))
    {
        tracing::warn!("cannot tell why joining failed: {error}");
    }
}

/// Opens the network's sign-in page, or an address the network redirects to it, and tells the person why when it cannot.
pub fn sign_in(network: &NetworkManager, activation_token: Option<&str>) {
    let page = match network.sign_in_page() {
        Ok(Some(page)) => Ok(page),
        Ok(None) => network.sign_in_probe(),
        Err(error) => {
            tracing::warn!("cannot ask for the sign-in page: {error}");
            network.sign_in_probe()
        }
    };
    let reason = match page {
        Ok(page) => match bande_xdg::open(&page, activation_token) {
            Ok(()) => return,
            Err(error) => error.to_string(),
        },
        Err(error) => error.to_string(),
    };
    tracing::warn!("cannot open the sign-in page: {reason}");
    let told = Connection::session().and_then(|session| {
        tell(
            &session,
            &tr!("The sign-in page could not be opened"),
            &reason,
        )
    });
    if let Err(error) = told {
        tracing::warn!("cannot tell why the sign-in page did not open: {error}");
    }
}

fn offer_sign_in(network: &NetworkManager, found: &Network) {
    if found.security != Security::Open {
        return;
    }
    match network.detects_sign_in_pages() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => tracing::debug!("cannot tell whether sign-in pages are detected: {error}"),
    }
    let mut offered = lock(&OFFERED);
    let summary = tr!("{network} may need you to sign in", network = found.name());
    let open = tr!("Open Sign-In Page");
    let posted = Connection::session().and_then(|session| {
        client::notify(
            &session,
            &Request {
                app_name: &tr!("Network"),
                replaces_id: offered.unwrap_or(0),
                app_icon: NETWORK_ICON,
                summary: &summary,
                body: &tr!("Open networks in hotels, cafés and airports often show a sign-in page before websites open."),
                actions: &[(DEFAULT_ACTION, ""), (OFFER_ACTION, &open)],
                urgency: Urgency::Normal,
                sound: None,
                desktop_entry: None,
            },
        )
    });
    match posted {
        Ok(id) => *offered = Some(id),
        Err(error) => tracing::warn!("cannot offer the network's sign-in page: {error}"),
    }
}

fn current_network(network: &NetworkManager) -> Option<String> {
    network
        .networks()
        .ok()?
        .into_iter()
        .find(|found| found.current)
        .map(|current| current.name())
}

struct Portal {
    system: Connection,
    network: NetworkManager,
    session: Connection,
    shown: Mutex<Option<u32>>,
}

impl Portal {
    fn check(&self, behind: &mut bool) {
        let portal = matches!(self.network.sign_in_page(), Ok(Some(_)));
        if portal == *behind {
            return;
        }
        *behind = portal;
        let mut shown = lock(&self.shown);
        if !portal {
            if let Some(id) = shown.take()
                && let Err(error) = client::close(&self.session, id)
            {
                tracing::debug!("the sign-in notice stays: {error}");
            }
            return;
        }
        let summary = sign_in_summary(current_network(&self.network).as_deref());
        let sign_in = tr!("Sign In");
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &tr!("Network"),
                replaces_id: shown.unwrap_or(0),
                app_icon: NETWORK_ICON,
                summary: &summary,
                body: &tr!("This network asks you to sign in before you can use the internet."),
                actions: &[(DEFAULT_ACTION, &sign_in)],
                urgency: Urgency::Normal,
                sound: None,
                desktop_entry: None,
            },
        );
        match posted {
            Ok(id) => *shown = Some(id),
            Err(error) => tracing::warn!("cannot ask to sign in to the network: {error}"),
        }
    }
}

fn watch_connectivity(portal: &Portal) -> zbus::Result<()> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(NETWORK_MANAGER)?
        .path(NETWORK_MANAGER_PATH)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .add_arg(NETWORK_MANAGER)?
        .build();
    let changes = MessageIterator::for_match_rule(rule, &portal.system, None)?;
    let mut behind = false;
    portal.check(&mut behind);
    for change in changes.flatten() {
        let connectivity = change
            .body()
            .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
            .is_ok_and(|(_, changed, invalidated)| {
                changed.contains_key(CONNECTIVITY)
                    || invalidated.iter().any(|name| name == CONNECTIVITY)
            });
        if connectivity {
            portal.check(&mut behind);
        }
    }
    Ok(())
}

fn notice_signals(session: &Connection) -> zbus::Result<MessageIterator> {
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .interface(NOTIFICATIONS)?
        .build();
    MessageIterator::for_match_rule(rule, session, None)
}

fn watch_notices(portal: &Portal, signals: MessageIterator) {
    let mut token: Option<(u32, String)> = None;
    for signal in signals.flatten() {
        let header = signal.header();
        let body = signal.body();
        match header.member().map(|member| member.as_str()) {
            Some("ActivationToken") => token = body.deserialize::<(u32, String)>().ok(),
            Some("ActionInvoked") => {
                let Ok((id, key)) = body.deserialize::<(u32, String)>() else {
                    continue;
                };
                let ours = (key == DEFAULT_ACTION && *lock(&portal.shown) == Some(id))
                    || ((key == DEFAULT_ACTION || key == OFFER_ACTION)
                        && *lock(&OFFERED) == Some(id));
                let token = token
                    .take()
                    .filter(|(given, _)| *given == id)
                    .map(|(_, token)| token);
                if ours {
                    sign_in(&portal.network, token.as_deref());
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _)) = body.deserialize::<(u32, u32)>() {
                    for notice in [&portal.shown, &OFFERED] {
                        let mut shown = lock(notice);
                        if *shown == Some(id) {
                            *shown = None;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn follow_captive_portal(system: &Connection, session: &Connection) {
    let portal = Arc::new(Portal {
        system: system.clone(),
        network: NetworkManager::new(system),
        session: session.clone(),
        shown: Mutex::new(None),
    });
    match notice_signals(session) {
        Ok(signals) => {
            let notices = Arc::clone(&portal);
            thread::spawn(move || watch_notices(&notices, signals));
        }
        Err(error) => tracing::warn!("sign-in notices cannot be clicked: {error}"),
    }
    thread::spawn(move || {
        if let Err(error) = watch_connectivity(&portal) {
            tracing::warn!("captive portals are not followed: {error}");
        }
    });
}
