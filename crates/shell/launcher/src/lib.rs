mod client;
mod clips;
mod emoji;
mod model;
mod preview;
mod rows;
mod tones;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use client::{LauncherShell, role};
pub use clips::{Clips, Reply, Request};
pub use model::{Action, AppInfo, Clip, Heading, Hit, Kind, Mode, PaneInfo, SettingInfo, Target};
pub use view::{CARD_WIDTH, Catalog, ICON_SIZE, Launcher, Loaded, Surface};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-launcher.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [
            include_str!("lib.rs"),
            include_str!("client.rs"),
            include_str!("view.rs"),
            include_str!("rows.rs"),
            include_str!("preview.rs"),
            include_str!("tones.rs"),
            include_str!("emoji.rs"),
            include_str!("model.rs"),
            include_str!("clips.rs"),
        ];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
