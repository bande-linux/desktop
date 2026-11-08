use resvg::{tiny_skia, usvg};
use smithay::input::pointer::CursorIcon;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Heading {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pointer {
    Kind(CursorIcon),
    Only(Heading),
}

impl From<CursorIcon> for Pointer {
    fn from(icon: CursorIcon) -> Self {
        Self::Kind(icon)
    }
}

pub(crate) const KINDS: [CursorIcon; 36] = [
    CursorIcon::Default,
    CursorIcon::ContextMenu,
    CursorIcon::Help,
    CursorIcon::Pointer,
    CursorIcon::Progress,
    CursorIcon::Wait,
    CursorIcon::Cell,
    CursorIcon::Crosshair,
    CursorIcon::Text,
    CursorIcon::VerticalText,
    CursorIcon::Alias,
    CursorIcon::Copy,
    CursorIcon::Move,
    CursorIcon::NoDrop,
    CursorIcon::NotAllowed,
    CursorIcon::Grab,
    CursorIcon::Grabbing,
    CursorIcon::EResize,
    CursorIcon::NResize,
    CursorIcon::NeResize,
    CursorIcon::NwResize,
    CursorIcon::SResize,
    CursorIcon::SeResize,
    CursorIcon::SwResize,
    CursorIcon::WResize,
    CursorIcon::EwResize,
    CursorIcon::NsResize,
    CursorIcon::NeswResize,
    CursorIcon::NwseResize,
    CursorIcon::ColResize,
    CursorIcon::RowResize,
    CursorIcon::AllScroll,
    CursorIcon::ZoomIn,
    CursorIcon::ZoomOut,
    CursorIcon::DndAsk,
    CursorIcon::AllResize,
];

pub(crate) const LEGACY_NAMES: [(&str, CursorIcon); 6] = [
    ("fleur", CursorIcon::AllScroll),
    ("dnd-copy", CursorIcon::Copy),
    ("dnd-link", CursorIcon::Alias),
    ("dnd-none", CursorIcon::NoDrop),
    ("dnd-move", CursorIcon::Default),
    ("left_ptr_help", CursorIcon::Help),
];

pub(crate) struct Image {
    pub(crate) side: u32,
    pub(crate) hotspot: (u32, u32),
    pub(crate) rgba: Vec<u8>,
}

impl Pointer {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Kind(icon) => icon.name(),
            Self::Only(Heading::North) => "resize-north-only",
            Self::Only(Heading::NorthEast) => "resize-north-east-only",
            Self::Only(Heading::East) => "resize-east-only",
            Self::Only(Heading::SouthEast) => "resize-south-east-only",
            Self::Only(Heading::South) => "resize-south-only",
            Self::Only(Heading::SouthWest) => "resize-south-west-only",
            Self::Only(Heading::West) => "resize-west-only",
            Self::Only(Heading::NorthWest) => "resize-north-west-only",
        }
    }

    fn source(self) -> &'static str {
        match self {
            Self::Kind(CursorIcon::ContextMenu | CursorIcon::DndAsk) => {
                include_str!("../data/cursors/context-menu.svg")
            }
            Self::Kind(CursorIcon::Help) => include_str!("../data/cursors/help.svg"),
            Self::Kind(CursorIcon::Pointer) => include_str!("../data/cursors/pointer.svg"),
            Self::Kind(CursorIcon::Progress) => include_str!("../data/cursors/progress.svg"),
            Self::Kind(CursorIcon::Wait) => include_str!("../data/cursors/wait.svg"),
            Self::Kind(CursorIcon::Cell) => include_str!("../data/cursors/cell.svg"),
            Self::Kind(CursorIcon::Crosshair) => include_str!("../data/cursors/crosshair.svg"),
            Self::Kind(CursorIcon::Text) => include_str!("../data/cursors/text.svg"),
            Self::Kind(CursorIcon::VerticalText) => {
                include_str!("../data/cursors/vertical-text.svg")
            }
            Self::Kind(CursorIcon::Alias) => include_str!("../data/cursors/alias.svg"),
            Self::Kind(CursorIcon::Copy) => include_str!("../data/cursors/copy.svg"),
            Self::Kind(CursorIcon::NoDrop | CursorIcon::NotAllowed) => {
                include_str!("../data/cursors/not-allowed.svg")
            }
            Self::Kind(CursorIcon::Grab) => include_str!("../data/cursors/grab.svg"),
            Self::Kind(CursorIcon::Grabbing) => include_str!("../data/cursors/grabbing.svg"),
            Self::Kind(CursorIcon::EResize | CursorIcon::WResize | CursorIcon::EwResize) => {
                include_str!("../data/cursors/ew-resize.svg")
            }
            Self::Kind(CursorIcon::NResize | CursorIcon::SResize | CursorIcon::NsResize) => {
                include_str!("../data/cursors/ns-resize.svg")
            }
            Self::Kind(CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize) => {
                include_str!("../data/cursors/nesw-resize.svg")
            }
            Self::Kind(CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize) => {
                include_str!("../data/cursors/nwse-resize.svg")
            }
            Self::Kind(CursorIcon::ColResize) => include_str!("../data/cursors/col-resize.svg"),
            Self::Kind(CursorIcon::RowResize) => include_str!("../data/cursors/row-resize.svg"),
            Self::Kind(CursorIcon::Move | CursorIcon::AllScroll | CursorIcon::AllResize) => {
                include_str!("../data/cursors/all-scroll.svg")
            }
            Self::Kind(CursorIcon::ZoomIn) => include_str!("../data/cursors/zoom-in.svg"),
            Self::Kind(CursorIcon::ZoomOut) => include_str!("../data/cursors/zoom-out.svg"),
            Self::Kind(_) => include_str!("../data/cursors/default.svg"),
            Self::Only(Heading::North) => include_str!("../data/cursors/resize-north-only.svg"),
            Self::Only(Heading::NorthEast) => {
                include_str!("../data/cursors/resize-north-east-only.svg")
            }
            Self::Only(Heading::East) => include_str!("../data/cursors/resize-east-only.svg"),
            Self::Only(Heading::SouthEast) => {
                include_str!("../data/cursors/resize-south-east-only.svg")
            }
            Self::Only(Heading::South) => include_str!("../data/cursors/resize-south-only.svg"),
            Self::Only(Heading::SouthWest) => {
                include_str!("../data/cursors/resize-south-west-only.svg")
            }
            Self::Only(Heading::West) => include_str!("../data/cursors/resize-west-only.svg"),
            Self::Only(Heading::NorthWest) => {
                include_str!("../data/cursors/resize-north-west-only.svg")
            }
        }
    }

    pub(crate) fn render(self, side: u32) -> Result<Image, String> {
        let source = self.source();
        let (x, y) = hotspot(source).ok_or("no data-hotspot")?;
        let tree = usvg::Tree::from_str(source, &usvg::Options::default())
            .map_err(|error| error.to_string())?;
        let mut pixmap = tiny_skia::Pixmap::new(side, side).ok_or("empty size")?;
        let scale = side as f32 / tree.size().width();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let place = |at: f32| ((at * scale).round().max(0.0) as u32).min(side - 1);
        Ok(Image {
            side,
            hotspot: (place(x), place(y)),
            rgba: pixmap.take(),
        })
    }
}

fn hotspot(source: &str) -> Option<(f32, f32)> {
    let (_, rest) = source.split_once("data-hotspot=\"")?;
    let (value, _) = rest.split_once('"')?;
    let (x, y) = value.split_once(' ')?;
    Some((x.parse().ok()?, y.parse().ok()?))
}
