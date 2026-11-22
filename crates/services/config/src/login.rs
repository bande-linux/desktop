use std::path::{Path, PathBuf};

use crate::{Accessibility, Config, Setting, TEXT_SIZE_RANGE};

pub const LOGIN_DEFAULTS: &str = "/etc/bande/login.kdl";
pub const CARRIED_ACCESSIBILITY: &str = "BANDE_LOGIN_ACCESSIBILITY";
const LOGIN_ACCESSIBILITY: &str = "bande-login-accessibility.kdl";

const SCREEN_READER: &str = "screen-reader";
const INCREASE_CONTRAST: &str = "increase-contrast";
const REDUCE_TRANSPARENCY: &str = "reduce-transparency";
const REDUCE_MOTION: &str = "reduce-motion";
const TEXT_SIZE: &str = "text-size=";

pub fn login_accessibility_file(runtime: &Path) -> PathBuf {
    runtime.join(LOGIN_ACCESSIBILITY)
}

pub fn accessibility_source(access: &Accessibility) -> String {
    Setting::Accessibility(*access)
        .apply("")
        .unwrap_or_default()
}

pub fn read_accessibility(path: &Path) -> Option<Accessibility> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", path.display());
            return None;
        }
    };
    Config::parse(&text)
        .inspect_err(|error| tracing::warn!("{} is not valid: {error}", path.display()))
        .ok()
        .map(|config| config.accessibility)
}

pub fn login_defaults() -> Accessibility {
    read_accessibility(Path::new(LOGIN_DEFAULTS)).unwrap_or_default()
}

/// The features `now` turns on beyond `before`, as the words of [`CARRIED_ACCESSIBILITY`].
pub fn carry(now: &Accessibility, before: &Accessibility) -> String {
    let flag = |on: bool, was: bool, word: &str| (on && !was).then(|| word.to_owned());
    [
        flag(now.screen_reader, before.screen_reader, SCREEN_READER),
        flag(
            now.increase_contrast,
            before.increase_contrast,
            INCREASE_CONTRAST,
        ),
        flag(
            now.reduce_transparency,
            before.reduce_transparency,
            REDUCE_TRANSPARENCY,
        ),
        flag(now.reduce_motion, before.reduce_motion, REDUCE_MOTION),
        (now.text_size > before.text_size).then(|| format!("{TEXT_SIZE}{}", now.text_size)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
}

/// Turns on in `into` the features `words` name, as [`carry`] writes them; fails on the first
/// word it does not know.
pub fn carried(words: &str, into: &mut Accessibility) -> Result<(), String> {
    for word in words.split_whitespace() {
        match word {
            SCREEN_READER => into.screen_reader = true,
            INCREASE_CONTRAST => into.increase_contrast = true,
            REDUCE_TRANSPARENCY => into.reduce_transparency = true,
            REDUCE_MOTION => into.reduce_motion = true,
            word => {
                let size = word
                    .strip_prefix(TEXT_SIZE)
                    .and_then(|size| size.parse::<u32>().ok())
                    .filter(|size| (TEXT_SIZE_RANGE.0..=TEXT_SIZE_RANGE.1).contains(size));
                match size {
                    Some(size) => into.text_size = into.text_size.max(size),
                    None => return Err(format!("unknown accessibility setting {word:?}")),
                }
            }
        }
    }
    Ok(())
}
