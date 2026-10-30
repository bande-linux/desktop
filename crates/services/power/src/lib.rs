pub mod adapter;
pub mod backlight;
pub mod battery;
pub mod controller;
pub mod idle;
pub mod logind;
pub mod screensaver;
pub mod wayland;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-power.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("main.rs"),
            include_str!("adapter.rs"),
            include_str!("backlight.rs"),
            include_str!("battery.rs"),
            include_str!("controller.rs"),
            include_str!("idle.rs"),
            include_str!("logind.rs"),
            include_str!("screensaver.rs"),
            include_str!("wayland.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
