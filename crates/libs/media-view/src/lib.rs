mod session;
mod stage;

pub use session::Session;
pub use stage::{Stage, StageWidget};

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub(crate) static L10N: LazyLock<Localizer> =
    LazyLock::new(|| Localizer::new(bande_l10n::translations!("bande-media-view")));
