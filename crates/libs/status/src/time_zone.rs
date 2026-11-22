use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use bande_datetime::daylight::ZONE_TAB;
use bande_datetime::timedate::TimedateProxyBlocking;
use bande_datetime::{ZoneSuggestion, city_name, suggested_zone};
use bande_l10n::tr;
use bande_notify::client::{self, Request, Signal};
use bande_notify::{DEFAULT_ACTION, Urgency};
use zbus::blocking::Connection;

const KEPT_FILE: &str = "bande/time-zone-kept";
const CHANGE_ACTION: &str = "change";
const CHOOSE_ACTION: &str = "choose";
const KEEP_ACTION: &str = "keep";
const SETTINGS_APP: &str = "org.bande.Settings";
const SETTINGS_PROGRAM: &str = "bande-settings";
const TIME_PANE: &str = "language-time";

struct Offer {
    id: u32,
    country: String,
    suggestion: ZoneSuggestion,
    token: Option<String>,
}

#[derive(Default)]
struct Offers {
    shown: Option<Offer>,
    dismissed: HashSet<String>,
}

type Offered = Arc<Mutex<Offers>>;

fn lock(offered: &Offered) -> MutexGuard<'_, Offers> {
    offered.lock().unwrap_or_else(PoisonError::into_inner)
}

fn kept_file() -> Option<PathBuf> {
    Some(bande_xdg::state_home()?.join(KEPT_FILE))
}

fn kept(country: &str) -> bool {
    kept_file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|kept| kept.lines().any(|line| line == country))
}

fn keep(country: &str) {
    let Some(path) = kept_file() else {
        return;
    };
    let changed = bande_xdg::change_file(&path, |kept| {
        let mut countries: Vec<&str> = kept.lines().collect();
        if !countries.contains(&country) {
            countries.push(country);
        }
        Ok(countries.iter().map(|line| format!("{line}\n")).collect())
    });
    if let Err(error) = changed {
        tracing::warn!("cannot remember the time zone kept: {error}");
    }
}

fn offer(session: &Connection, offered: &Offered) {
    let country = match bande_network::wifi_country() {
        Ok(Some(country)) => country,
        Ok(None) => return,
        Err(error) => {
            tracing::debug!("cannot read the Wi-Fi country: {error}");
            return;
        }
    };
    let mut offers = lock(offered);
    if kept(&country)
        || offers.dismissed.contains(&country)
        || offers
            .shown
            .as_ref()
            .is_some_and(|shown| shown.country == country)
    {
        return;
    }
    let zone_tab = match std::fs::read_to_string(ZONE_TAB) {
        Ok(zone_tab) => zone_tab,
        Err(error) => {
            tracing::warn!("cannot read {ZONE_TAB}: {error}");
            return;
        }
    };
    let now = bande_datetime::now();
    let Some(suggestion) = suggested_zone(&zone_tab, &country, now.time_zone(), now.timestamp())
    else {
        return;
    };
    let (summary, action) = match &suggestion {
        ZoneSuggestion::Zone(zone) => (
            tr!("Change the time zone to {city}?", city = city_name(zone)),
            (CHANGE_ACTION, tr!("Change")),
        ),
        ZoneSuggestion::Choose => (
            tr!("Change the time zone?"),
            (CHOOSE_ACTION, tr!("Choose…")),
        ),
    };
    let keep = tr!("Keep Current");
    let posted = client::notify(
        session,
        &Request {
            app_name: &tr!("Date & Time"),
            replaces_id: offers.shown.as_ref().map_or(0, |shown| shown.id),
            app_icon: SETTINGS_APP,
            summary: &summary,
            body: &tr!(
                "The Wi-Fi networks nearby are in a country where the time differs from this computer’s clock."
            ),
            actions: &[
                (DEFAULT_ACTION, ""),
                (action.0, &action.1),
                (KEEP_ACTION, &keep),
            ],
            urgency: Urgency::Normal,
            sound: None,
            desktop_entry: Some(SETTINGS_APP),
        },
    );
    match posted {
        Ok(id) => {
            offers.shown = Some(Offer {
                id,
                country,
                suggestion,
                token: None,
            });
        }
        Err(error) => tracing::warn!("cannot suggest a time zone: {error}"),
    }
}

fn change_zone(system: &Connection, zone: String) {
    let system = system.clone();
    thread::spawn(move || {
        let changed = TimedateProxyBlocking::new(&system)
            .and_then(|timedate| timedate.set_timezone(&zone, true));
        if let Err(error) = changed {
            tracing::warn!("cannot change the time zone to {zone}: {error}");
        }
    });
}

fn answer(system: &Connection, offered: &Offered, signal: Signal) {
    let mut offers = lock(offered);
    let (id, action) = match signal {
        Signal::ActionInvoked(id, action) => (id, Some(action)),
        Signal::Closed(id) => (id, None),
        Signal::ActivationToken(id, token) => {
            if let Some(shown) = offers.shown.as_mut().filter(|shown| shown.id == id) {
                shown.token = Some(token);
            }
            return;
        }
    };
    if offers.shown.as_ref().is_none_or(|shown| shown.id != id) {
        return;
    }
    let Some(offer) = offers.shown.take() else {
        return;
    };
    match (action.as_deref(), offer.suggestion) {
        (None, _) => {
            offers.dismissed.insert(offer.country);
        }
        (Some(CHANGE_ACTION), ZoneSuggestion::Zone(zone)) => change_zone(system, zone),
        (Some(CHOOSE_ACTION | DEFAULT_ACTION), _) => {
            let command = [SETTINGS_PROGRAM, TIME_PANE];
            let token = offer.token.as_deref();
            if let Err(error) = bande_xdg::spawn_detached(SETTINGS_APP, &command, None, token) {
                tracing::warn!("cannot open the time zone settings: {error}");
            }
        }
        (Some(KEEP_ACTION), _) => keep(&offer.country),
        _ => {}
    }
}

pub fn suggest_time_zones(system: &Connection, session: &Connection) {
    let offered: Offered = Arc::default();
    let answers = (system.clone(), session.clone(), Arc::clone(&offered));
    thread::spawn(move || {
        let (system, session, offered) = answers;
        let listened = client::listen(&session, |signal| {
            answer(&system, &offered, signal);
            true
        });
        if let Err(error) = listened {
            tracing::warn!("time zone suggestions cannot be answered: {error}");
        }
    });
    let (system, session) = (system.clone(), session.clone());
    thread::spawn(move || {
        let followed = bande_network::follow_online(&system, |online| {
            if online {
                offer(&session, &offered);
            }
            true
        });
        if let Err(error) = followed {
            tracing::warn!("no time zone suggestions: {error}");
        }
    });
}
