use accesskit::{Action, ActionRequest, Node, Role};

use super::control::{Symbol, focus_ring};
use super::{Context, Event, Key, Size, Widget, WidgetId};
use bande_graphics::{
    Color, ControlSize, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextStyle,
};

pub const BUTTON_HEIGHT: f32 = ControlSize::Regular.height();
const PADDING: f32 = 16.0;
const FONT_SIZE: f32 = 13.0;
const SYMBOL_SIZE: f32 = 16.0;
const HOVER_INK: f32 = 0.08;
const PRESSED_INK: f32 = 0.16;
const PRESSED_SHADE: f32 = 0.15;

pub struct Button {
    id: WidgetId,
    title: String,
    on_click: Box<dyn FnMut()>,
    pressed: bool,
    hovered: bool,
    role: ButtonRole,
    tooltip: Option<String>,
    paragraph: Option<Paragraph>,
    symbol: Option<Symbol>,
    enabled: bool,
    frame: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonRole {
    Plain,
    Primary,
    Cancel,
    Quiet,
    Destructive,
}

impl Button {
    pub fn new(title: &str, on_click: impl FnMut() + 'static) -> Self {
        Self {
            id: WidgetId::next(),
            title: title.to_owned(),
            on_click: Box::new(on_click),
            pressed: false,
            hovered: false,
            role: ButtonRole::Plain,
            tooltip: None,
            paragraph: None,
            symbol: None,
            enabled: true,
            frame: Rect::default(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.pressed = false;
            self.hovered = false;
        }
    }

    pub fn symbol(symbol: Image, label: &str, on_click: impl FnMut() + 'static) -> Self {
        Self {
            symbol: Some(Symbol::new(symbol)),
            tooltip: Some(label.to_owned()),
            ..Self::new(label, on_click)
        }
    }

    pub fn primary(self) -> Self {
        Self {
            role: ButtonRole::Primary,
            ..self
        }
    }

    pub fn cancel(self) -> Self {
        Self {
            role: ButtonRole::Cancel,
            ..self
        }
    }

    pub fn quiet(self) -> Self {
        Self {
            role: ButtonRole::Quiet,
            ..self
        }
    }

    pub fn destructive(self) -> Self {
        Self {
            role: ButtonRole::Destructive,
            ..self
        }
    }

    pub fn tooltip(self, tooltip: &str) -> Self {
        Self {
            tooltip: Some(tooltip.to_owned()),
            ..self
        }
    }

    fn text_style(&self, cx: &Context) -> TextStyle {
        let palette = &cx.palette;
        if !self.enabled {
            return TextStyle::new(FONT_SIZE, palette.ink3).weight(TextStyle::MEDIUM);
        }
        match self.role {
            ButtonRole::Primary => {
                TextStyle::new(FONT_SIZE, palette.on_accent).weight(TextStyle::SEMIBOLD)
            }
            ButtonRole::Destructive => {
                TextStyle::new(FONT_SIZE, palette.on_danger).weight(TextStyle::MEDIUM)
            }
            ButtonRole::Quiet => {
                TextStyle::new(FONT_SIZE, palette.accent_text).weight(TextStyle::SEMIBOLD)
            }
            ButtonRole::Plain | ButtonRole::Cancel => {
                TextStyle::new(FONT_SIZE, palette.ink).weight(TextStyle::MEDIUM)
            }
        }
    }

    fn fill(&self, cx: &Context) -> Option<Color> {
        let palette = &cx.palette;
        let shaded = |color: Color| match self.pressed {
            true => color.mix(Color::BLACK, PRESSED_SHADE),
            false => color,
        };
        let inked = |color: Color| match (self.pressed, self.hovered) {
            (true, _) => color.mix(palette.ink, PRESSED_INK),
            (false, true) => color.mix(palette.ink, HOVER_INK),
            (false, false) => color,
        };
        if !self.enabled {
            return self.symbol.is_none().then_some(palette.well);
        }
        match self.role {
            ButtonRole::Primary => Some(shaded(palette.accent)),
            ButtonRole::Destructive => Some(shaded(palette.danger)),
            ButtonRole::Plain | ButtonRole::Cancel if self.symbol.is_none() => {
                Some(inked(palette.well))
            }
            _ => (self.hovered || self.pressed).then(|| inked(palette.well)),
        }
    }
}

impl Widget for Button {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        if let Some(symbol) = &mut self.symbol {
            symbol.tint(match (self.enabled, self.hovered || self.pressed) {
                (false, _) => cx.palette.ink3,
                (true, true) => cx.palette.ink,
                (true, false) => cx.palette.ink2,
            });
            let side = cx.control_height(ControlSize::Regular);
            return Size::new(side, side);
        }
        let style = self.text_style(cx);
        let paragraph = cx
            .text
            .paragraph(&self.title, style, None, TextAlign::Start);
        let size = Size::new(
            (paragraph.width() + 2.0 * PADDING).ceil(),
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
        if let Some(fill) = self.fill(cx) {
            items.push(Item::Fill {
                shape,
                paint: fill.into(),
            });
        }
        if cx.is_focused(self.id) {
            focus_ring(cx, shape, items);
        }
        if let Some(symbol) = &self.symbol {
            symbol.paint(
                Rect::new(
                    self.frame.x + ((self.frame.width - SYMBOL_SIZE) / 2.0).round(),
                    self.frame.y + ((self.frame.height - SYMBOL_SIZE) / 2.0).round(),
                    SYMBOL_SIZE,
                    SYMBOL_SIZE,
                ),
                items,
            );
        }
        if let Some(paragraph) = self.paragraph.as_ref().filter(|_| self.symbol.is_none()) {
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
        node.set_label(self.title.clone());
        if self.enabled {
            node.add_action(Action::Click);
        } else {
            node.set_disabled();
        }
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        if !self.enabled {
            return false;
        }
        match event {
            Event::PointerEnter(_) => self.hovered = true,
            Event::PointerLeave => {
                self.hovered = false;
                self.pressed = false;
            }
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerMove(pointer) if self.pressed => {
                self.pressed = self.frame.contains(pointer.position);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let activate = self.pressed && self.frame.contains(pointer.position);
                self.pressed = false;
                if activate {
                    (self.on_click)();
                }
            }
            Event::Key {
                key: Key::Space, ..
            } => (self.on_click)(),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if self.enabled && request.action == Action::Click {
            (self.on_click)();
            return true;
        }
        false
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        self.enabled && cx.keyboard_navigation
    }

    fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    fn key_equivalent(&self) -> Option<Key> {
        match self.role {
            ButtonRole::Primary => Some(Key::Enter),
            ButtonRole::Cancel => Some(Key::Escape),
            ButtonRole::Plain | ButtonRole::Quiet | ButtonRole::Destructive => None,
        }
    }
}
