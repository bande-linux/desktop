pub mod agent;
pub mod dialog;
pub mod helper;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/unbound-polkit.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
