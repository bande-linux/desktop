mod model;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use model::{
    Accept, Chooser, glob_matches, initial_folder, initial_name, remember, remembered_folder,
};
pub use view::{ChooserView, Finished, Icons};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-file-chooser.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("view.rs"), include_str!("main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
