mod layouts;
mod shortcuts;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use layouts::{EVDEV_RULES, Layout, Layouts};
pub use shortcuts::{Action, Area, SHORTCUTS, Shortcut, shortcuts_in};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-keyboard.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("shortcuts.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
