pub mod dock;
pub mod menubar;
pub mod menus;
pub mod rail;
mod symbols;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-bar.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("dock.rs"),
            include_str!("menubar.rs"),
            include_str!("menus.rs"),
            include_str!("rail.rs"),
            include_str!("combined_client.rs"),
            include_str!("dialogs.rs"),
            include_str!("dock_client.rs"),
            include_str!("menu_bar_client.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
