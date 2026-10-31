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
