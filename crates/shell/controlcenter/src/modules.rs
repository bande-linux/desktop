use std::path::PathBuf;

use accesskit::{Action, ActionRequest, Node, Role, Toggled};
use bande_toolkit::ui::{Context, Event, Key, Size, Widget, WidgetId, focus_ring_for};
use bande_toolkit::{
    Image, Item, Palette, Paragraph, Point, Radius, Rect, Scheme, Shape, Spacing, TextAlign,
    TextRole, TextStyle,
};
use bande_xdg::{ICON_THEMES, find_icon};

use crate::{Change, Changes};

pub const MODULE_RADIUS: f32 = Radius::L;
pub const MODULE_PADDING: f32 = Spacing::M as f32;
pub const TILE_HEIGHT: f32 = 52.0;
const DISC: f32 = 28.0;
const GLYPH: f32 = 16.0;
const GLYPH_PIXELS: u32 = 48;
const HOVER_INK: f32 = 0.04;
const PRESSED_INK: f32 = 0.08;

pub fn symbol(name: &str, icon_dirs: &[PathBuf]) -> Option<Image> {
    let path = find_icon(name, GLYPH_PIXELS, &ICON_THEMES, icon_dirs)?;
    Image::load(&path, GLYPH_PIXELS)
}

pub fn module_surface(palette: &Palette, rect: Rect, fill: f32, items: &mut Vec<Item>) {
    let shape = Shape::rounded(rect, MODULE_RADIUS);
    items.push(Item::Fill {
        shape,
        paint: palette.sheet.mix(palette.ink, fill).into(),
    });
    if palette.scheme == Scheme::Dark {
        items.push(Item::Stroke {
            shape: Shape::rounded(rect.inset(0.5), MODULE_RADIUS - 0.5),
            width: 1.0,
            paint: palette.hairline.into(),
        });
    }
}

pub struct Module {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    frame: Rect,
}

impl Module {
    pub fn new(content: Box<dyn Widget>) -> Self {
        Self {
            id: WidgetId::next(),
            children: vec![content],
            frame: Rect::default(),
        }
    }
}

impl Widget for Module {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let inner = Size::new(
            available.width - 2.0 * MODULE_PADDING,
            available.height - 2.0 * MODULE_PADDING,
        );
        let content = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, inner).height)
            .sum::<f32>();
        Size::new(available.width, content + 2.0 * MODULE_PADDING)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        for child in &mut self.children {
            child.place(cx, frame.inset(MODULE_PADDING));
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        module_surface(&cx.palette, self.frame, 0.0, items);
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::Group)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Caption {
    id: WidgetId,
    text: String,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Caption {
    pub fn new(text: &str) -> Self {
        Self {
            id: WidgetId::next(),
            text: text.to_owned(),
            paragraph: None,
            frame: Rect::default(),
        }
    }
}

impl Widget for Caption {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let paragraph = cx.text.paragraph(
            &self.text,
            TextStyle::role(TextRole::Label, cx.palette.ink3),
            Some(available.width),
            TextAlign::Start,
        );
        let height = paragraph.height().ceil();
        self.paragraph = Some(paragraph);
        Size::new(available.width, height)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, _cx: &Context, items: &mut Vec<Item>) {
        if let Some(paragraph) = &self.paragraph {
            items.push(Item::Text {
                origin: Point::new(self.frame.x, self.frame.y),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Label);
        node.set_value(self.text.clone());
        node
    }
}

pub struct TileSpec {
    pub title: String,
    pub detail: String,
    pub symbol: &'static str,
    pub on: bool,
    pub toggle: fn(bool) -> Change,
    pub details: Option<Change>,
}

pub struct Tile {
    id: WidgetId,
    spec: TileSpec,
    width: f32,
    changes: Changes,
    glyph: Option<Image>,
    tinted: Option<Image>,
    title: Option<Paragraph>,
    detail: Option<Paragraph>,
    hovered: bool,
    pressed: bool,
    frame: Rect,
}

impl Tile {
    pub fn new(spec: TileSpec, width: f32, icon_dirs: &[PathBuf], changes: &Changes) -> Self {
        Self {
            id: WidgetId::next(),
            glyph: symbol(spec.symbol, icon_dirs),
            spec,
            width,
            changes: Changes::clone(changes),
            tinted: None,
            title: None,
            detail: None,
            hovered: false,
            pressed: false,
            frame: Rect::default(),
        }
    }

    fn disc(&self) -> Rect {
        Rect::new(
            self.frame.x + MODULE_PADDING,
            self.frame.y + ((TILE_HEIGHT - DISC) / 2.0).round(),
            DISC,
            DISC,
        )
    }

    fn toggle(&mut self) {
        self.spec.on = !self.spec.on;
        self.changes
            .borrow_mut()
            .push((self.spec.toggle)(self.spec.on));
    }

    fn show_details(&self) -> bool {
        match &self.spec.details {
            Some(change) if self.spec.on => {
                self.changes.borrow_mut().push(change.clone());
                true
            }
            _ => false,
        }
    }

    fn activate_at(&mut self, point: Point) {
        if self.disc().inset(-(Spacing::XS as f32)).contains(point) || !self.show_details() {
            self.toggle();
        }
    }
}

impl Widget for Tile {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let palette = &cx.palette;
        let room = self.width - 2.0 * MODULE_PADDING - DISC - Spacing::S as f32;
        let fit = |text: &mut bande_toolkit::TextSystem, line: &str, style: TextStyle| {
            crate::now_playing::fitted(text, line, style, room)
        };
        self.title = Some(fit(
            &mut cx.text,
            &self.spec.title,
            TextStyle::role(TextRole::BodyStrong, palette.ink),
        ));
        self.detail = (!self.spec.detail.is_empty()).then(|| {
            fit(
                &mut cx.text,
                &self.spec.detail,
                TextStyle::role(TextRole::Callout, palette.ink2),
            )
        });
        let tint = if self.spec.on {
            palette.on_accent
        } else {
            palette.ink2
        };
        self.tinted = self.glyph.as_ref().and_then(|glyph| glyph.tinted(tint));
        Size::new(self.width, TILE_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, self.width, TILE_HEIGHT);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let palette = &cx.palette;
        let ink = match (self.pressed, self.hovered) {
            (true, _) => PRESSED_INK,
            (false, true) => HOVER_INK,
            (false, false) => 0.0,
        };
        module_surface(palette, self.frame, ink, items);
        if cx.is_focused(self.id) {
            focus_ring_for(
                &cx.palette,
                Shape::rounded(self.frame, MODULE_RADIUS),
                items,
            );
        }
        let disc = self.disc();
        items.push(Item::Fill {
            shape: Shape::rounded(disc, DISC / 2.0),
            paint: if self.spec.on {
                palette.accent
            } else {
                palette.well
            }
            .into(),
        });
        if let Some(glyph) = &self.tinted {
            items.push(Item::Image {
                rect: Rect::new(
                    disc.x + (DISC - GLYPH) / 2.0,
                    disc.y + (DISC - GLYPH) / 2.0,
                    GLYPH,
                    GLYPH,
                ),
                image: glyph.clone(),
            });
        }
        let left = disc.x + DISC + Spacing::S as f32;
        let lines: Vec<&Paragraph> = self.title.iter().chain(&self.detail).collect();
        let height: f32 = lines.iter().map(|line| line.height()).sum();
        let mut top = (self.frame.y + (TILE_HEIGHT - height) / 2.0).round();
        for line in lines {
            items.push(Item::Text {
                origin: Point::new(left, top),
                paragraph: line.clone(),
            });
            top += line.height();
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Switch);
        node.set_label(self.spec.title.clone());
        node.set_description(self.spec.detail.clone());
        node.set_toggled(if self.spec.on {
            Toggled::True
        } else {
            Toggled::False
        });
        node.add_action(Action::Click);
        if self.spec.details.is_some() {
            node.add_action(Action::Expand);
        }
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
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
                    self.activate_at(pointer.position);
                }
            }
            Event::Key {
                key: Key::Space, ..
            } => self.toggle(),
            Event::Key {
                key: Key::Enter | Key::Right,
                ..
            } => return self.show_details(),
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        match request.action {
            Action::Click => self.toggle(),
            Action::Expand => return self.show_details(),
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}
