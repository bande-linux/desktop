use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_network::NetworkManager;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::OwnedValue;
use zbus::{MatchRule, message};

use crate::L10N;

const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const NOTIFICATIONS: &str = "org.freedesktop.Notifications";
const CONNECTIVITY: &str = "Connectivity";
const SIGN_IN_ACTION: &str = "default";
const NETWORK_ICON: &str = "network-wireless-symbolic";

fn lock(shown: &Mutex<Option<u32>>) -> MutexGuard<'_, Option<u32>> {
    shown.lock().unwrap_or_else(PoisonError::into_inner)
}

fn sign_in_summary(network: Option<&str>) -> String {
    match network {
        Some(network) => L10N.tr_args("sign-in-to", [("network", network.to_owned().into())]),
        None => L10N.tr("sign-in-to-network"),
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
        let sign_in = L10N.tr("sign-in");
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &L10N.tr("network"),
                replaces_id: shown.unwrap_or(0),
                app_icon: NETWORK_ICON,
                summary: &summary,
                body: &L10N.tr("sign-in-body"),
                actions: &[(SIGN_IN_ACTION, &sign_in)],
                urgency: Urgency::Normal,
                sound: None,
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

fn watch_notices(portal: &Portal, signals: MessageIterator, open: &dyn Fn(&str)) {
    for signal in signals.flatten() {
        let header = signal.header();
        let body = signal.body();
        match header.member().map(|member| member.as_str()) {
            Some("ActionInvoked") => {
                if let Ok((id, key)) = body.deserialize::<(u32, String)>()
                    && key == SIGN_IN_ACTION
                    && *lock(&portal.shown) == Some(id)
                    && let Ok(Some(page)) = portal.network.sign_in_page()
                {
                    open(&page);
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _)) = body.deserialize::<(u32, u32)>() {
                    let mut shown = lock(&portal.shown);
                    if *shown == Some(id) {
                        *shown = None;
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn follow_captive_portal(
    system: &Connection,
    session: &Connection,
    open: impl Fn(&str) + Send + 'static,
) {
    let portal = Arc::new(Portal {
        system: system.clone(),
        network: NetworkManager::new(system),
        session: session.clone(),
        shown: Mutex::new(None),
    });
    match notice_signals(session) {
        Ok(signals) => {
            let notices = Arc::clone(&portal);
            thread::spawn(move || watch_notices(&notices, signals, &open));
        }
        Err(error) => tracing::warn!("sign-in notices cannot be clicked: {error}"),
    }
    thread::spawn(move || {
        if let Err(error) = watch_connectivity(&portal) {
            tracing::warn!("captive portals are not followed: {error}");
        }
    });
}

pub fn open_page(page: &str) {
    let command = ["xdg-open".to_owned(), page.to_owned()];
    if let Err(error) = bande_xdg::spawn_detached("", &command, None, None) {
        tracing::warn!("cannot open the sign-in page: {error}");
    }
}
