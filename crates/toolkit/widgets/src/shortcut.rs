use accesskit::{Action, ActionRequest, Node, Role};
use bande_graphics::{
    ControlSize, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextStyle, TextSystem,
};
use bande_l10n::{Text, text};

use super::control::{focus_ring, focus_ring_for};
use super::{Context, Event, Key, Modifiers, Size, Widget, WidgetId};

const MODIFIER_NAMES: [(&[&str], Text); 4] = [
    (&["Shift"], text!("Shift")),
    (&["Ctrl", "Control"], text!("Ctrl")),
    (&["Alt"], text!("Alt")),
    (&["Super", "Logo", "Meta"], text!("Super")),
];

const KEY_NAMES: [(&[&str], Text, Key); 16] = [
    (&["Left"], text!("←"), Key::Left),
    (&["Right"], text!("→"), Key::Right),
    (&["Up"], text!("↑"), Key::Up),
    (&["Down"], text!("↓"), Key::Down),
    (&["Backspace"], text!("Backspace"), Key::Backspace),
    (
        &["Delete", "Del"],
        text!(context = "key", "Delete"),
        Key::Delete,
    ),
    (&["Enter", "Return"], text!("Enter"), Key::Enter),
    (&["Tab"], text!("Tab"), Key::Tab),
    (&["Escape", "Esc"], text!("Esc"), Key::Escape),
    (&["Space"], text!("Space"), Key::Space),
    (&["Home"], text!(context = "key", "Home"), Key::Home),
    (&["End"], text!("End"), Key::End),
    (&["PageUp", "PgUp"], text!("Page Up"), Key::PageUp),
    (&["PageDown", "PgDn"], text!("Page Down"), Key::PageDown),
    (&["Insert"], text!(context = "key", "Insert"), Key::Insert),
    (&["Menu"], text!("Menu"), Key::Menu),
];

const BODY_SIZE: f32 = 13.0;
const FIELD_PADDING: f32 = 16.0;

fn split(shortcut: &str) -> (Vec<&str>, &str) {
    let parts: Vec<&str> = shortcut.split('+').collect();
    match parts.split_last() {
        Some((&"", rest)) if rest.last() == Some(&"") => (rest[..rest.len() - 1].to_vec(), "+"),
        Some((key, rest)) => (rest.to_vec(), *key),
        None => (Vec::new(), ""),
    }
}

fn key_name(key: &str) -> Option<&'static (&'static [&'static str], Text, Key)> {
    KEY_NAMES
        .iter()
        .find(|(names, _, _)| names.iter().any(|name| name.eq_ignore_ascii_case(key)))
}

pub fn shortcut_label(shortcut: &str) -> String {
    let (modifiers, key) = split(shortcut);
    let key = match key_name(key) {
        Some((_, id, _)) => id.tr(),
        None if key.chars().count() == 1 => key.chars().flat_map(char::to_uppercase).collect(),
        None => key.to_owned(),
    };
    MODIFIER_NAMES
        .iter()
        .filter(|(names, _)| {
            modifiers
                .iter()
                .any(|modifier| names.iter().any(|name| name.eq_ignore_ascii_case(modifier)))
        })
        .map(|(_, id)| id.tr())
        .chain([key])
        .collect::<Vec<_>>()
        .join("+")
}

pub fn parse_shortcut(shortcut: &str) -> Option<(Key, Modifiers)> {
    let (names, key) = split(shortcut);
    let mut modifiers = Modifiers::default();
    for name in names {
        let held = MODIFIER_NAMES
            .iter()
            .find(|(names, _)| names.iter().any(|known| known.eq_ignore_ascii_case(name)))
            .and_then(|(names, _)| names.first().copied())?;
        match held {
            "Shift" => modifiers.shift = true,
            "Ctrl" => modifiers.control = true,
            "Alt" => modifiers.alt = true,
            _ => modifiers.logo = true,
        }
    }
    let function = key
        .strip_prefix(['F', 'f'])
        .and_then(|number| number.parse::<u8>().ok())
        .filter(|number| (1..=24).contains(number));
    let key = match (key_name(key), function) {
        (Some((_, _, named)), _) => *named,
        (None, Some(number)) => Key::Function(number),
        (None, None) => {
            let mut characters = key.chars();
            let (Some(character), None) = (characters.next(), characters.next()) else {
                return None;
            };
            Key::Character(match modifiers.shift {
                true => character.to_ascii_uppercase(),
                false => character.to_ascii_lowercase(),
            })
        }
    };
    Some((key, modifiers))
}

pub fn shortcut_pressed(event: &Event, shortcut: &str) -> bool {
    matches!(event, Event::Key { key, modifiers } if parse_shortcut(shortcut) == Some((*key, *modifiers)))
}

#[derive(Debug, Clone)]
pub struct ShortcutText {
    label: Paragraph,
}

impl ShortcutText {
    pub fn new(text: &mut TextSystem, shortcut: &str, style: TextStyle) -> Self {
        Self {
            label: text.paragraph(&shortcut_label(shortcut), style, None, TextAlign::Start),
        }
    }

    pub fn width(&self) -> f32 {
        self.label.width()
    }

    pub fn height(&self) -> f32 {
        self.label.height()
    }

    pub fn paint(&self, origin: Point, items: &mut Vec<Item>) {
        items.push(Item::Text {
            origin,
            paragraph: self.label.clone(),
        });
    }
}

pub struct Shortcut {
    id: WidgetId,
    shortcut: String,
    text: Option<ShortcutText>,
    frame: Rect,
}

impl Shortcut {
    pub fn new(shortcut: &str) -> Self {
        Self {
            id: WidgetId::next(),
            shortcut: shortcut.to_owned(),
            text: None,
            frame: Rect::default(),
        }
    }
}

impl Widget for Shortcut {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let text = ShortcutText::new(
            &mut cx.text,
            &self.shortcut,
            TextStyle::new(BODY_SIZE, cx.palette.ink2),
        );
        let size = Size::new(text.width().ceil(), text.height().ceil());
        self.text = Some(text);
        size
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        if let Some(text) = &self.text {
            text.paint(Point::new(self.frame.x, self.frame.y), items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_label(shortcut_label(&self.shortcut));
        node.set_keyboard_shortcut(shortcut_label(&self.shortcut));
        node
    }
}

fn chord_text(key: Key, modifiers: Modifiers) -> String {
    let held = [
        (modifiers.shift, "Shift"),
        (modifiers.control, "Ctrl"),
        (modifiers.alt, "Alt"),
        (modifiers.logo, "Super"),
    ];
    let key = match (KEY_NAMES.iter().find(|(_, _, named)| *named == key), key) {
        (Some((names, _, _)), _) => names[0].to_owned(),
        (None, Key::Function(number)) => format!("F{number}"),
        (None, Key::Character(character)) => character.to_uppercase().collect(),
        (None, _) => String::new(),
    };
    held.into_iter()
        .filter(|(held, _)| *held)
        .map(|(_, name)| name.to_owned())
        .chain([key])
        .collect::<Vec<_>>()
        .join("+")
}

/// A button that listens for the next chord when pressed, for choosing a shortcut's keys.
pub struct ShortcutField {
    id: WidgetId,
    label: String,
    shortcut: Option<String>,
    listening: bool,
    on_record: Box<dyn FnMut(&str) -> bool>,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl ShortcutField {
    /// `on_record` gets each chord pressed while listening, such as `Ctrl+Alt+M`, and
    /// says whether the field takes it.
    pub fn new(
        label: &str,
        shortcut: Option<&str>,
        on_record: impl FnMut(&str) -> bool + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            shortcut: shortcut.map(str::to_owned),
            listening: false,
            on_record: Box::new(on_record),
            paragraph: None,
            frame: Rect::default(),
        }
    }

    fn shown(&self) -> String {
        match (&self.shortcut, self.listening) {
            (_, true) => text!("Press Keys…").tr(),
            (Some(shortcut), false) => shortcut_label(shortcut),
            (None, false) => text!("Choose Keys…").tr(),
        }
    }

    fn record(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if key == Key::Escape && modifiers == Modifiers::default() {
            self.listening = false;
            return true;
        }
        let held = Modifiers {
            shift: false,
            ..modifiers
        };
        if held == Modifiers::default() {
            return false;
        }
        let chord = chord_text(key, modifiers);
        if (self.on_record)(&chord) {
            self.shortcut = Some(chord);
            self.listening = false;
        }
        true
    }
}

impl Widget for ShortcutField {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let paragraph = cx.text.paragraph(
            &self.shown(),
            TextStyle::new(BODY_SIZE, cx.palette.ink).weight(TextStyle::MEDIUM),
            None,
            TextAlign::Start,
        );
        let size = Size::new(
            (paragraph.width() + 2.0 * FIELD_PADDING).ceil(),
            cx.control_height(ControlSize::Regular),
        );
        self.paragraph = Some(paragraph);
        size
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let shape = Shape::rounded(self.frame, Radius::S);
        items.push(Item::Fill {
            shape,
            paint: cx.palette.well.into(),
        });
        if self.listening {
            focus_ring_for(&cx.palette, shape, items);
        } else if cx.is_focused(self.id) {
            focus_ring(cx, shape, items);
        }
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(
                    self.frame.x + ((self.frame.width - paragraph.width()) / 2.0).round(),
                    self.frame.y + ((self.frame.height - paragraph.height()) / 2.0).round(),
                ),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Button);
        node.set_label(self.label.clone());
        node.set_value(self.shown());
        if let Some(shortcut) = self.shortcut.as_deref().filter(|_| !self.listening) {
            node.set_keyboard_shortcut(shortcut_label(shortcut));
        }
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } if self.listening => self.record(*key, *modifiers),
            Event::PointerUp(pointer)
                if pointer.is_primary() && self.frame.contains(pointer.position) =>
            {
                self.listening = !self.listening;
                true
            }
            Event::Key {
                key: Key::Space,
                modifiers,
            } if *modifiers == Modifiers::default() => {
                self.listening = true;
                true
            }
            _ => false,
        }
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action == Action::Click {
            self.listening = !self.listening;
            return true;
        }
        false
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
