//! Account offers: the account setup of one app hands a server and its sign-in to another app
//! that speaks the other protocol (Calendar's CalDAV account to Contacts, or Contacts'
//! CardDAV account to Calendar), which finds its collections there and adopts the account.
//! The password waits in the Secret Service, never on disk.

use std::path::PathBuf;

use bande_discovery::{Resolver, domain_of};
use bande_keyring::Keyring;
use serde::{Deserialize, Serialize};

use crate::client::{Client, Service};
use crate::http::{Credentials, Error, host, secure};

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
    let program = format!("/usr/bin/bande-{}", service.app());
    let keyring = Keyring::shared(KEYRING, vec![program]).map_err(|error| error.to_string())?;
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
            let offer: Offer = serde_json::from_str(&text)
                .inspect_err(|error| tracing::warn!("cannot read an account offer: {error}"))
                .ok()?;
            (file(service, &offer.id)? == entry.path()).then_some(offer)
        })
        .collect()
}

fn password(offer: &Offer) -> Result<Option<String>, String> {
    Keyring::open(KEYRING)
        .and_then(|keyring| keyring.password(&[("offer", &offer.id)]))
        .map_err(|error| error.to_string())
}

/// Whether the domain of `address` vouches for `server`, so that a forged DNS answer cannot
/// choose who gets the password (RFC 6764 section 8).
fn vouched(address: &str, server: &str) -> bool {
    let Some((domain, host)) = domain_of(address).zip(host(server)) else {
        return false;
    };
    secure(server) && (host == domain || host.ends_with(&format!(".{domain}")))
}

/// Signs in to each waiting offer for `service` and runs `discover` there: on the offered
/// server, else on the one the offer's address points to (RFC 6764) inside the address's own
/// domain. Each answered offer names the server that answered. An offer whose server or
/// password cannot be reached now waits for the next call; every other one is answered and
/// withdrawn. Blocks on the network and the Secret Service.
pub fn adopt<T>(
    service: Service,
    discover: impl Fn(&Client, &str) -> Result<T, Error>,
) -> Vec<(Offer, Credentials, Result<T, Error>)> {
    let mut answered = Vec::new();
    for mut offer in waiting(service) {
        let password = match password(&offer) {
            Ok(Some(password)) => password,
            Ok(None) => {
                tracing::warn!("the password offered for {} is gone", offer.address);
                withdraw(service, &offer.id);
                continue;
            }
            Err(error) => {
                tracing::warn!("the offered account waits for the keyring: {error}");
                continue;
            }
        };
        let credentials = Credentials {
            user: offer.user.clone(),
            password,
        };
        let client = Client::new(service, Some(&credentials));
        let result = discover(&client, &offer.server).or_else(|error| {
            let found = Client::find(service, &offer.address, &Resolver::system())
                .filter(|found| *found != offer.server && vouched(&offer.address, found));
            match found {
                Some(found) => {
                    let result = discover(&client, &found)?;
                    offer.server = found;
                    Ok(result)
                }
                None => Err(error),
            }
        });
        if let Err(Error::Network(error)) = &result {
            tracing::info!("the offered account waits for the network: {error}");
            continue;
        }
        withdraw(service, &offer.id);
        answered.push((offer, credentials, result));
    }
    answered
}

/// Removes the offer `id` to the app that speaks `service` and its password, once adopted,
/// refused or no longer wanted.
pub fn withdraw(service: Service, id: &str) {
    if let Err(error) = Keyring::open(KEYRING).and_then(|keyring| keyring.forget(&[("offer", id)]))
    {
        tracing::warn!("cannot forget an offered password: {error}");
    }
    if let Some(path) = file(service, id)
        && let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::vouched;

    #[test]
    fn an_offered_password_follows_dns_only_inside_the_address_domain() {
        for (server, trusted) in [
            ("https://example.org/dav/", true),
            ("https://carddav.example.org:8443/", true),
            ("https://example.org.evil.net/", false),
            ("https://notexample.org/", false),
            ("https://evil.net?.example.org/", false),
            ("https://evil.net#.example.org/", false),
            ("https://example.org@evil.net/", false),
            ("http://carddav.example.org/", false),
        ] {
            assert_eq!(vouched("me@example.org", server), trusted, "{server}");
        }
        assert!(!vouched("me", "https://example.org/"));
    }
}
