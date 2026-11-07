mod session;
mod stage;

pub use session::Session;
pub use stage::{Stage, StageWidget};

use std::sync::LazyLock;

use bande_l10n::Localizer;

const ENGLISH: &str = include_str!("../locales/en-US/bande-media-view.ftl");
pub(crate) static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
