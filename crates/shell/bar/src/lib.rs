mod bar_client;
pub mod dialogs;
mod installed_apps;
pub mod low_memory;
pub mod menus;
pub mod pairing;
pub mod rail;
mod status_client;
pub mod surfaces;
pub mod symbols;
pub mod taskbar;
mod taskbar_client;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-bar")));
