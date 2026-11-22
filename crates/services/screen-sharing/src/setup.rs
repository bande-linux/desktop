use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// The user unit that runs the server; Screen Sharing is on while it is enabled.
pub const UNIT: &str = "bande-screen-sharing.service";
/// The program the unit runs, the only one the screenshot tool shows the screen to.
pub const PROGRAM: &str = "/usr/libexec/bande-screen-sharing";
/// The TCP port of RFB (RFC 6143).
pub const PORT: u16 = 5900;
/// Viewers use the first 8 characters with VNC authentication, so passwords are at least that long.
pub const SHORTEST_PASSWORD: usize = 8;
/// Whose keyring item holds the password.
pub const KEYRING_APPLICATION: &str = "bande-screen-sharing";
/// The attributes of the keyring item that holds the password.
pub const PASSWORD_KEY: [(&str, &str); 1] = [("service", "screen-sharing")];

/// The folder with this computer's certificate and key.
pub fn folder() -> Option<PathBuf> {
    bande_xdg::data_home().map(|data| data.join("bande").join("screen-sharing"))
}

/// The DER certificate viewers see when they connect.
pub fn certificate_file() -> Option<PathBuf> {
    folder().map(|folder| folder.join("certificate.der"))
}

/// The SHA-256 fingerprint of a DER certificate, as viewers show it: hex pairs split by colons.
pub fn fingerprint(certificate: &[u8]) -> String {
    Sha256::digest(certificate)
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}
