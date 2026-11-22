use bande_l10n::tr;
use sequoia_openpgp::cert::{Cert, CertParser};
use sequoia_openpgp::parse::Parse;
use sequoia_openpgp::serialize::Serialize;

use crate::error::{Error, Result};
use crate::net::Network;

const ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
const MAX_KEY: usize = 1 << 20;

fn zbase32(bytes: &[u8]) -> String {
    let mut text = String::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            text.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        text.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    text
}

fn percent(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

pub fn urls(email: &str) -> Option<[String; 2]> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    let domain = domain.to_ascii_lowercase();
    let valid = !local.is_empty()
        && !domain.is_empty()
        && domain
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-".contains(character));
    if !valid {
        return None;
    }
    let hash = ring::digest::digest(
        &ring::digest::SHA1_FOR_LEGACY_USE_ONLY,
        local.to_lowercase().as_bytes(),
    );
    let hash = zbase32(hash.as_ref());
    let local = percent(local);
    Some([
        format!("https://openpgpkey.{domain}/.well-known/openpgpkey/{domain}/hu/{hash}?l={local}"),
        format!("https://{domain}/.well-known/openpgpkey/hu/{hash}?l={local}"),
    ])
}

pub fn certificate_for(data: &[u8], email: &str) -> Option<Cert> {
    CertParser::from_bytes(data)
        .ok()?
        .filter_map(std::result::Result::ok)
        .find(|cert| {
            cert.userids().any(|user_id| {
                user_id
                    .userid()
                    .email_normalized()
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some(email)
            })
        })
}

pub fn look_up(email: &str, network: &Network) -> Result<Vec<u8>> {
    let email = email.trim().to_lowercase();
    let urls =
        urls(&email).ok_or_else(|| Error::Crypto(tr!("This address can’t be looked up.")))?;
    let mut last = None;
    for url in urls {
        match crate::http::get(&url, "application/octet-stream", network, MAX_KEY) {
            Ok(data) => match certificate_for(&data, &email) {
                Some(cert) => {
                    let cert = cert.retain_userids(|user_id| {
                        user_id
                            .userid()
                            .email_normalized()
                            .ok()
                            .flatten()
                            .as_deref()
                            == Some(email.as_str())
                    });
                    let mut binary = Vec::new();
                    cert.serialize(&mut binary)
                        .map_err(|error| Error::Crypto(error.to_string()))?;
                    return Ok(binary);
                }
                None => {
                    last = Some(Error::Crypto(tr!(
                        "The key found for {email} is not for that address.",
                        email = email.clone()
                    )))
                }
            },
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| Error::Crypto(tr!("No key was found."))))
}
