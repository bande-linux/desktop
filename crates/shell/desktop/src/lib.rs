mod backdrop;
mod model;
mod places;
mod view;

use std::sync::LazyLock;

use unbound_l10n::Localizer;

pub use backdrop::{Key as WallpaperKey, Layer, Luma, placeholder};
pub use model::{
    Bounds, CELL_HEIGHT, CELL_WIDTH, Desktop, Direction, GRID_PADDING, Grid, ICON_SIZE, Item, Kind,
    Positions, Slot, SortBy, icon_bounds, label_bounds,
};
pub use places::{
    Layouts, format_layouts, layouts_path, list, media_roots, mounted_volumes, parse_layouts,
    read_item, save_layouts,
};
pub use view::{DesktopView, Request, fitted_label};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-desktop-surface.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("view.rs"), include_str!("../src/main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
