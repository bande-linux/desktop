mod model;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use model::{
    Accept, Chooser, glob_matches, initial_folder, initial_name, remember, remembered_folder,
    typed_location,
};
pub use view::{ChooserView, Finished, Icons};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-file-chooser.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
