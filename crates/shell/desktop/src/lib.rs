mod backdrop;
mod client;
mod confirm;
mod menus;
mod model;
mod places;
mod view;

use std::sync::LazyLock;

use bande_l10n::Localizer;

pub use backdrop::{Key as WallpaperKey, Layer, Luma};
pub use client::{DesktopSurface, confirm, confirm_extension, report_eject_failure, role};
pub use confirm::{CONFIRM_DELETE_FLAG, CONFIRM_EXTENSION_FLAG, EJECT_FAILED_FLAG, confirm_delete};
pub use menus::{App as MenuApp, Command, Selection, command, desktop_menu, items_menu};
pub use model::{
    Bounds, CELL_HEIGHT, CELL_WIDTH, Desktop, Direction, GRID_PADDING, Grid, ICON_SIZE, Item, Kind,
    Positions, Slot, SortBy, icon_bounds, label_bounds,
};
pub use places::{
    Layouts, format_layouts, layouts_path, list, parse_layouts, read_item, save_layouts,
};
pub use view::{Artwork, DesktopView, Request, fitted_label};

const ENGLISH: &str = include_str!("../locales/en-US/bande-desktop-surface.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));
