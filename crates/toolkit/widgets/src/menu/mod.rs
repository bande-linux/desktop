mod model;
mod panel;
mod tracker;

use accesskit::NodeId;
use unbound_graphics::{Radius, TextSize, grown_line};

pub use self::model::{Menu, MenuItem, Search};
pub use self::panel::MenuPanel;
pub use self::tracker::{MenuOutcome, MenuTracker};

const ROW_HEIGHT: f32 = 28.0;
pub(crate) const VERTICAL_PADDING: f32 = 4.0;
pub(crate) const TEXT_START: f32 = 28.0;
pub const MENU_RADIUS: f32 = Radius::L;
pub const MENU_SHADOW_MARGIN: f32 = 24.0;

pub(crate) fn row_height(text: TextSize) -> f32 {
    ROW_HEIGHT + grown_line(text)
}

const MENU_NODES: u64 = u64::MAX - 1024;
const SEARCH_NODE: NodeId = NodeId(MENU_NODES + 1);

fn menu_node(level: usize, index: Option<usize>) -> NodeId {
    let level = level as u64 * 256;
    NodeId(MENU_NODES - level - index.map_or(0, |index| index as u64 + 1))
}
