use accesskit::{Node, Role};
use unbound_graphics::{
    Color, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use super::{Context, Size, Widget, WidgetId};

const MODIFIER_SYMBOLS: [(&[&str], char); 3] = [
    (&["Ctrl", "Control"], '⌃'),
    (&["Alt", "Option"], '⌥'),
    (&["Shift"], '⇧'),
];
const SUPER_NAMES: [&str; 3] = ["Super", "Logo", "Meta"];

const KEY_SYMBOLS: [(&str, &str); 14] = [
    ("Backspace", "⌫"),
    ("Delete", "⌦"),
    ("Return", "↩"),
    ("Enter", "↩"),
    ("Escape", "⎋"),
    ("Tab", "⇥"),
    ("Left", "←"),
    ("Right", "→"),
    ("Up", "↑"),
    ("Down", "↓"),
    ("PageUp", "⇞"),
    ("PageDown", "⇟"),
    ("Home", "↖"),
    ("End", "↘"),
];

const BODY_SIZE: f32 = 13.0;
const GLYPH: f32 = 9.5;
const GLYPH_RADIUS: f32 = 2.5;
const GLYPH_CENTRE: f32 = 3.0;
const GLYPH_STROKE: f32 = 1.1;
const GLYPH_GAP: f32 = 1.5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutKeys {
    pub modifiers: String,
    pub logo: bool,
    pub key: String,
}

pub fn shortcut_keys(shortcut: &str) -> ShortcutKeys {
    let parts: Vec<&str> = shortcut.split('+').collect();
    let (key, modifiers) = match parts.split_last() {
        Some((&"", rest)) if rest.last() == Some(&"") => ("+", &rest[..rest.len() - 1]),
        Some((key, rest)) => (*key, rest),
        None => ("", &[][..]),
    };
    let named = |names: &[&str]| {
        modifiers
            .iter()
            .any(|modifier| names.iter().any(|name| name.eq_ignore_ascii_case(modifier)))
    };
    let key = match KEY_SYMBOLS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
    {
        Some((_, symbol)) => (*symbol).to_owned(),
        None if key.chars().count() == 1 => key.chars().flat_map(char::to_uppercase).collect(),
        None => key.to_owned(),
    };
    ShortcutKeys {
        modifiers: MODIFIER_SYMBOLS
            .iter()
            .filter(|(names, _)| named(names))
            .map(|(_, symbol)| *symbol)
            .collect(),
        logo: named(&SUPER_NAMES),
        key,
    }
}

#[derive(Debug, Clone)]
pub struct ShortcutText {
    modifiers: Option<Paragraph>,
    logo: Option<f32>,
    key: Paragraph,
}

impl ShortcutText {
    pub fn new(text: &mut TextSystem, shortcut: &str, style: TextStyle) -> Self {
        let keys = shortcut_keys(shortcut);
        let mut paragraph = |content: &str| text.paragraph(content, style, None, TextAlign::Start);
        Self {
            modifiers: (!keys.modifiers.is_empty()).then(|| paragraph(&keys.modifiers)),
            logo: keys.logo.then_some(GLYPH * style.size / BODY_SIZE),
            key: paragraph(&keys.key),
        }
    }

    pub fn width(&self) -> f32 {
        self.modifiers.as_ref().map_or(0.0, Paragraph::width)
            + self.logo.map_or(0.0, |glyph| glyph + 2.0 * GLYPH_GAP)
            + self.key.width()
    }

    pub fn height(&self) -> f32 {
        self.key.height()
    }

    pub fn paint(&self, origin: Point, color: Color, items: &mut Vec<Item>) {
        let mut x = origin.x;
        if let Some(modifiers) = &self.modifiers {
            items.push(Item::Text {
                origin: Point::new(x, origin.y),
                paragraph: modifiers.clone(),
            });
            x += modifiers.width();
        }
        if let Some(glyph) = self.logo {
            x += GLYPH_GAP;
            let scale = glyph / GLYPH;
            let top = origin.y + ((self.key.height() - glyph) / 2.0).round();
            let outline = Rect::new(x, top, glyph, glyph);
            let stroke = GLYPH_STROKE * scale;
            items.push(Item::Stroke {
                shape: Shape::rounded(
                    outline.inset(stroke / 2.0),
                    GLYPH_RADIUS * scale - stroke / 2.0,
                ),
                width: stroke,
                paint: color.into(),
            });
            let centre = GLYPH_CENTRE * scale;
            items.push(Item::Fill {
                shape: Shape::rect(Rect::new(
                    x + (glyph - centre) / 2.0,
                    top + (glyph - centre) / 2.0,
                    centre,
                    centre,
                )),
                paint: color.into(),
            });
            x += glyph + GLYPH_GAP;
        }
        items.push(Item::Text {
            origin: Point::new(x, origin.y),
            paragraph: self.key.clone(),
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

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if let Some(text) = &self.text {
            text.paint(
                Point::new(self.frame.x, self.frame.y),
                cx.palette.ink2,
                items,
            );
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_label(self.shortcut.clone());
        node.set_keyboard_shortcut(self.shortcut.clone());
        node
    }
}

#[cfg(test)]
mod tests {
    use super::{ShortcutKeys, shortcut_keys};

    fn keys(modifiers: &str, logo: bool, key: &str) -> ShortcutKeys {
        ShortcutKeys {
            modifiers: modifiers.to_owned(),
            logo,
            key: key.to_owned(),
        }
    }

    #[test]
    fn shortcuts_read_as_key_symbols_in_the_standard_modifier_order() {
        assert_eq!(shortcut_keys("Super+Q"), keys("", true, "Q"));
        assert_eq!(shortcut_keys("Shift+Super+N"), keys("⇧", true, "N"));
        assert_eq!(
            shortcut_keys("Super+Ctrl+Alt+Shift+z"),
            keys("⌃⌥⇧", true, "Z")
        );
        assert_eq!(shortcut_keys("Control+Shift+q"), keys("⌃⇧", false, "Q"));
        assert_eq!(shortcut_keys("Alt+Super+Backspace"), keys("⌥", true, "⌫"));
        assert_eq!(shortcut_keys("Ctrl+Super+Left"), keys("⌃", true, "←"));
        assert_eq!(shortcut_keys("Super++"), keys("", true, "+"));
        assert_eq!(shortcut_keys("Shift+Super+."), keys("⇧", true, "."));
        assert_eq!(shortcut_keys("F2"), keys("", false, "F2"));
        assert_eq!(shortcut_keys("Space"), keys("", false, "Space"));
    }
}
