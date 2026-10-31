mod combined_client;
pub mod dialogs;
pub mod dock;
mod dock_client;
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

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-bar.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
