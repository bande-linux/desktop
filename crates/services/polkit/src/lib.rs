pub mod agent;
pub mod bridge;
pub mod dialog;
pub mod helper;

use std::sync::LazyLock;

use bande_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-polkit.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
