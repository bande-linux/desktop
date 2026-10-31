mod registry;
mod search;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use registry::{
    AUTOMATIC, Choice, DIM_CHOICES, Group, Kind, LOCK_CHOICES, List, MAX_SEGMENTS, POINTER_SPEEDS,
    Pane, PaneId, Permission, Section, Setting, SettingId, TEXT_SIZES, Value, WITH_DISPLAY,
    duration_label, panes,
};
pub use search::{PaneHit, SettingHit, search};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-settings-panes.ftl");

static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("registry.rs"), include_str!("search.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
