use crate::scene::Point;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub logo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Backspace,
    Delete,
    Enter,
    Tab,
    Escape,
    Space,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Menu,
    Function(u8),
    Character(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointerButton {
    #[default]
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pointer {
    pub position: Point,
    pub button: PointerButton,
    pub count: u8,
    pub modifiers: Modifiers,
}

impl Pointer {
    pub const fn at(position: Point) -> Self {
        Self {
            position,
            button: PointerButton::Primary,
            count: 1,
            modifiers: Modifiers {
                shift: false,
                alt: false,
                control: false,
                logo: false,
            },
        }
    }

    pub const fn with_button(self, button: PointerButton) -> Self {
        Self { button, ..self }
    }

    pub const fn with_count(self, count: u8) -> Self {
        Self { count, ..self }
    }

    pub const fn with_modifiers(self, modifiers: Modifiers) -> Self {
        Self { modifiers, ..self }
    }

    pub fn is_primary(&self) -> bool {
        self.button == PointerButton::Primary
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollSource {
    #[default]
    Wheel,
    Finger,
    Continuous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollPhase {
    Began,
    #[default]
    Changed,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scroll {
    pub position: Point,
    pub delta: Point,
    pub source: ScrollSource,
    pub phase: ScrollPhase,
    pub modifiers: Modifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
    Crosshair,
    Move,
    Grab,
    Grabbing,
    NotAllowed,
    ContextMenu,
    Copy,
    Alias,
    ColumnResize,
    RowResize,
    ResizeTop,
    ResizeBottom,
    ResizeLeft,
    ResizeRight,
    ResizeTopLeft,
    ResizeTopRight,
    ResizeBottomLeft,
    ResizeBottomRight,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    PointerDown(Pointer),
    PointerMove(Pointer),
    PointerUp(Pointer),
    PointerEnter(Pointer),
    PointerLeave,
    Scroll(Scroll),
    ModifiersChanged(Modifiers),
    Key {
        key: Key,
        modifiers: Modifiers,
    },
    Commit(String),
    Preedit {
        text: String,
        cursor: Option<(usize, usize)>,
    },
}
