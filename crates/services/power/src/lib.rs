pub mod adapter;
pub mod backlight;
pub mod battery;
pub mod controller;
pub mod idle;
pub mod logind;
pub mod screensaver;
pub mod wayland;

use std::sync::LazyLock;

use bande_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-power.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
