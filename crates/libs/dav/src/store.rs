use std::io::Read;
use std::path::Path;

/// A new random UID (an RFC 9562 version 4 UUID).
pub fn new_uid() -> String {
    let mut bytes = [0u8; 16];
    let read =
        std::fs::File::open("/dev/urandom").and_then(|mut random| random.read_exact(&mut bytes));
    if read.is_err() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos())
            .to_le_bytes();
        bytes.copy_from_slice(&now);
        bytes[0] ^= std::process::id() as u8;
    }
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The address of the resource with `uid` in the collection at `url` (or, with no URL, its
/// name in a local collection).
pub fn href_for(url: Option<&str>, uid: &str, extension: &str) -> String {
    let safe: String = uid
        .chars()
        .map(
            |character| match character.is_ascii_alphanumeric() || "-_.".contains(character) {
                true => character,
                false => '_',
            },
        )
        .collect();
    match url {
        Some(url) => format!("{}/{safe}.{extension}", url.trim_end_matches('/')),
        None => format!("{safe}.{extension}"),
    }
}

/// The JSON in `file`, or nothing when there is none; a file that cannot be read is renamed
/// to `.broken` and kept, so the offline copy starts over without losing it.
pub fn read_or_set_aside<T: serde::de::DeserializeOwned>(file: &Path) -> Option<T> {
    let text = std::fs::read_to_string(file).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(error) => {
            let aside = file.with_extension("broken");
            tracing::warn!(
                "cannot read {}: {error}; keeping it as {}",
                file.display(),
                aside.display()
            );
            if let Err(error) = std::fs::rename(file, &aside) {
                tracing::warn!("cannot keep {}: {error}", aside.display());
            }
            None
        }
    }
}
