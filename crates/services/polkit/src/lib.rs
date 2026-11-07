pub mod agent;
pub mod bridge;
pub mod dialog;
pub mod helper;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-polkit")));
