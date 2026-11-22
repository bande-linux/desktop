//! Account offers: the account setup of one app hands a server and its sign-in to another app
//! that speaks the other protocol (Calendar's CalDAV account to Contacts, or Contacts'
//! CardDAV account to Calendar), which finds its collections there and adopts the account.
//! The password waits in the Secret Service, never on disk.

use std::path::PathBuf;

use bande_keyring::Keyring;
use serde::{Deserialize, Serialize};

use crate::client::{Client, Service};
use crate::http::{Credentials, Error};

const KEYRING: &str = "org.bande.AccountOffers";
const FOLDER: &str = "bande/account-offers";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    pub id: String,
    pub address: String,
    pub server: String,
    pub user: String,
}

fn folder() -> Option<PathBuf> {
    bande_xdg::data_home().map(|home| home.join(FOLDER))
}

fn prefix(service: Service) -> &'static str {
    match service {
        Service::CalDav => "caldav-",
        Service::CardDav => "carddav-",
    }
}

fn file(service: Service, id: &str) -> Option<PathBuf> {
    let safe: String = id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .collect();
    Some(folder()?.join(format!("{}{safe}.json", prefix(service))))
}

/// Offers the account to the app that speaks `service`. Blocks on the Secret Service.
pub fn make(service: Service, offer: &Offer, password: &str) -> Result<(), String> {
    let path = file(service, &offer.id).ok_or("there is no home folder")?;
    let keyring = Keyring::open(KEYRING).map_err(|error| error.to_string())?;
    keyring
        .save(&[("offer", &offer.id)], &offer.user, password)
        .map_err(|error| error.to_string())?;
    let text = serde_json::to_string(offer).map_err(|error| error.to_string())?;
    bande_xdg::replace_private_file(&path, &text).map_err(|error| error.to_string())
}

/// The accounts other apps offered to the app that speaks `service`.
pub fn waiting(service: Service) -> Vec<Offer> {
    let Some(folder) = folder() else {
        return Vec::new();
    };
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(prefix(service)) && name.ends_with(".json"))
        })
        .filter_map(|entry| {
            let text = std::fs::read_to_string(entry.path())
                .inspect_err(|error| tracing::warn!("cannot read an account offer: {error}"))
                .ok()?;
            serde_json::from_str(&text)
                .inspect_err(|error| tracing::warn!("cannot read an account offer: {error}"))
                .ok()
        })
        .collect()
}

/// The password of `offer`. Blocks on the Secret Service.
pub fn password(offer: &Offer) -> Option<String> {
    Keyring::open(KEYRING)
        .and_then(|keyring| keyring.password(&[("offer", &offer.id)]))
        .inspect_err(|error| tracing::warn!("cannot read an offered password: {error}"))
        .ok()
        .flatten()
}

/// Signs in to each waiting offer for `service` and runs `discover` there: on the offered
/// server, else on the one the offer's address points to (RFC 6764). An offer whose server
/// cannot be reached now waits for the next call; every other one is answered and withdrawn.
/// Blocks on the network and the Secret Service.
pub fn adopt<T>(
    service: Service,
    discover: impl Fn(&Client, &str) -> Result<T, Error>,
) -> Vec<(Offer, Credentials, Result<T, Error>)> {
    let mut answered = Vec::new();
    for offer in waiting(service) {
        let Some(password) = password(&offer) else {
            withdraw(service, &offer);
            continue;
        };
        let credentials = Credentials {
            user: offer.user.clone(),
            password,
        };
        let client = Client::new(service, Some(&credentials));
        let result = discover(&client, &offer.server).or_else(|error| {
            match Client::find(
                service,
                &offer.address,
                &bande_discovery::Resolver::system(),
            ) {
                Some(found) if found != offer.server => discover(&client, &found),
                _ => Err(error),
            }
        });
        if let Err(Error::Network(error)) = &result {
            tracing::info!("the offered account waits for the network: {error}");
            continue;
        }
        withdraw(service, &offer);
        answered.push((offer, credentials, result));
    }
    answered
}

/// Removes `offer` and its password, once adopted or refused.
pub fn withdraw(service: Service, offer: &Offer) {
    if let Err(error) =
        Keyring::open(KEYRING).and_then(|keyring| keyring.forget(&[("offer", &offer.id)]))
    {
        tracing::warn!("cannot forget an offered password: {error}");
    }
    if let Some(path) = file(service, &offer.id)
        && let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove {}: {error}", path.display());
    }
}
