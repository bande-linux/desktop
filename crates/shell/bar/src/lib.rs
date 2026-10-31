mod combined_client;
pub mod dialogs;
pub mod dock;
mod dock_client;
mod installed_apps;
pub mod low_memory;
pub mod menu_bar;
mod menu_bar_client;
pub mod menu_search;
mod menu_search_client;
pub mod menus;
pub mod pairing;
pub mod rail;
pub mod surfaces;
pub mod symbols;

use std::sync::LazyLock;

use bande_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-bar.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("dock.rs"),
            include_str!("menu_bar.rs"),
            include_str!("menus.rs"),
            include_str!("menu_search.rs"),
            include_str!("rail.rs"),
            include_str!("combined_client.rs"),
            include_str!("dialogs.rs"),
            include_str!("low_memory.rs"),
            include_str!("dock_client.rs"),
            include_str!("menu_bar_client.rs"),
            include_str!("pairing.rs"),
        ];
        assert_eq!(
            bande_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
