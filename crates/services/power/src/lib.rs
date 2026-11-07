pub mod adapter;
pub mod audio;
pub mod backlight;
pub mod battery;
pub mod brightness;
pub mod controller;
pub mod ddc;
pub mod idle;
pub mod logind;
pub mod password;
pub mod screensaver;
pub mod wayland;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-power")));
