pub mod agent;
pub mod dialog;
pub mod helper;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-polkit.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("dialog.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
