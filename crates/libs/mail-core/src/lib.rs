pub mod accounts;
pub mod discover;
pub mod error;
pub mod http;
pub mod imap;
pub mod mime;
pub mod net;
pub mod setup;
pub mod sheet;
pub mod smtp;
#[cfg(feature = "testing")]
pub mod testing;

pub const KEYRING: &str = "org.bande.Mail";

pub fn keyring_key<'a>(email: &'a str, host: &'a str) -> [(&'a str, &'a str); 2] {
    [("account", email), ("server", host)]
}

pub fn keyring_label(email: &str) -> String {
    format!("Mail password for {email}")
}
