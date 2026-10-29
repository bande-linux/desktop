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
    Function(u8),
    Character(char),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    PointerDown(Point),
    PointerMove(Point),
    PointerUp(Point),
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
