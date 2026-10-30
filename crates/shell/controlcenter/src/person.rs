use std::path::PathBuf;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_toolkit::ui::{Context, Event, Parts, Size, Widget, WidgetId};
use unbound_toolkit::{Image, Item, Paragraph, Point, Rect, Shape, TextAlign, TextStyle};
use unbound_xdg::{ICON_THEMES, find_icon};

use crate::{Change, Changes, L10N};

pub const HEIGHT: f32 = 32.0;
const AVATAR: f32 = 28.0;
const NAME_GAP: f32 = 10.0;
const BUTTON: f32 = 28.0;
const BUTTON_GAP: f32 = 4.0;
const GLYPH: f32 = 16.0;
const GLYPH_PIXELS: u32 = 48;
const NAME_SIZE: f32 = 14.0;
const INITIAL_SIZE: f32 = 13.0;
const HOVER_ALPHA: u8 = 22;

const BUTTONS: [(&str, &str); 3] = [
    ("lock-screen", "system-lock-screen-symbolic"),
    ("power", "system-shutdown-symbolic"),
    ("system-settings", "emblem-system-symbolic"),
];

fn change(index: usize) -> Option<Change> {
    match index {
        0 => Some(Change::Lock),
        1 => Some(Change::Power),
        2 => Some(Change::Settings),
        _ => None,
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

pub struct PersonRow {
    id: WidgetId,
    name: String,
    icon_dirs: Vec<PathBuf>,
    changes: Changes,
    hovered: Option<usize>,
    name_text: Option<Paragraph>,
    initial: Option<Paragraph>,
    glyphs: Vec<Option<Image>>,
    frame: Rect,
}

impl PersonRow {
    pub fn new(name: &str, icon_dirs: Vec<PathBuf>, changes: &Changes) -> Self {
        Self {
            id: WidgetId::next(),
            name: name.to_owned(),
            icon_dirs,
            changes: changes.clone(),
            hovered: None,
            name_text: None,
            initial: None,
            glyphs: Vec::new(),
            frame: Rect::default(),
        }
    }

    fn button_rect(&self, index: usize) -> Rect {
        let right = self.frame.x + self.frame.width;
        let left = right - (BUTTONS.len() - index) as f32 * (BUTTON + BUTTON_GAP) + BUTTON_GAP;
        Rect::new(left, self.frame.y + (HEIGHT - BUTTON) / 2.0, BUTTON, BUTTON)
    }

    fn button_at(&self, point: Point) -> Option<usize> {
        (0..BUTTONS.len()).find(|index| contains(self.button_rect(*index), point))
    }

    fn press(&self, index: usize) {
        if let Some(change) = change(index) {
            self.changes.borrow_mut().push(change);
        }
    }
}

impl Widget for PersonRow {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, HEIGHT)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let ink = cx.palette.ink;
        let room = self.button_rect(0).x - frame.x - AVATAR - 2.0 * NAME_GAP;
        let style = TextStyle::new(NAME_SIZE, ink).weight(TextStyle::SEMIBOLD);
        let mut name: Vec<char> = self.name.chars().collect();
        let mut shown = self.name.clone();
        self.name_text = loop {
            let paragraph = cx.text.paragraph(&shown, style, None, TextAlign::Start);
            if paragraph.width() <= room || name.len() <= 1 {
                break Some(paragraph);
            }
            name.pop();
            shown = format!("{}…", name.iter().collect::<String>().trim_end());
        };
        let initial: String = self
            .name
            .chars()
            .next()
            .map(|first| first.to_uppercase().collect())
            .unwrap_or_default();
        self.initial = Some(cx.text.paragraph(
            &initial,
            TextStyle::new(INITIAL_SIZE, ink).weight(TextStyle::SEMIBOLD),
            None,
            TextAlign::Start,
        ));
        let dirs = &self.icon_dirs;
        self.glyphs = BUTTONS
            .iter()
            .map(|(_, glyph)| {
                let path = find_icon(glyph, GLYPH_PIXELS, &ICON_THEMES, dirs)?;
                Image::load_symbolic(&path, GLYPH_PIXELS, ink)
            })
            .collect();
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let avatar = Rect::new(
            self.frame.x,
            self.frame.y + (HEIGHT - AVATAR) / 2.0,
            AVATAR,
            AVATAR,
        );
        items.push(Item::Fill {
            shape: Shape::rounded(avatar, AVATAR / 2.0),
            paint: cx.palette.well.into(),
        });
        if let Some(initial) = &self.initial {
            items.push(Item::Text {
                origin: Point::new(
                    (avatar.x + (AVATAR - initial.width()) / 2.0).round(),
                    (avatar.y + (AVATAR - initial.height()) / 2.0).round(),
                ),
                paragraph: initial.clone(),
            });
        }
        if let Some(name) = &self.name_text {
            items.push(Item::Text {
                origin: Point::new(
                    avatar.x + AVATAR + NAME_GAP,
                    (self.frame.y + (HEIGHT - name.height()) / 2.0).round(),
                ),
                paragraph: name.clone(),
            });
        }
        for index in 0..BUTTONS.len() {
            let rect = self.button_rect(index);
            let fill = if self.hovered == Some(index) {
                cx.palette.ink.with_alpha(HOVER_ALPHA)
            } else {
                cx.palette.well
            };
            items.push(Item::Fill {
                shape: Shape::rounded(rect, BUTTON / 2.0),
                paint: fill.into(),
            });
            if let Some(Some(glyph)) = self.glyphs.get(index) {
                items.push(Item::Image {
                    rect: Rect::new(
                        rect.x + (BUTTON - GLYPH) / 2.0,
                        rect.y + (BUTTON - GLYPH) / 2.0,
                        GLYPH,
                        GLYPH,
                    ),
                    image: glyph.clone(),
                });
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Group);
        node.set_label(self.name.clone());
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerMove(pointer) => {
                let hovered = self.button_at(pointer.position);
                let changed = hovered != self.hovered;
                self.hovered = hovered;
                changed
            }
            Event::PointerLeave => self.hovered.take().is_some(),
            Event::PointerDown(pointer) if pointer.is_primary() => {
                match self.button_at(pointer.position) {
                    Some(index) => {
                        self.press(index);
                        true
                    }
                    None => false,
                }
            }
            _ => false,
        }
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<_> = BUTTONS
            .iter()
            .enumerate()
            .map(|(index, (label, _))| {
                let mut node = Node::new(Role::Button);
                node.set_label(L10N.tr(label));
                node.add_action(Action::Click);
                let rect = self.button_rect(index);
                node.set_bounds(accesskit::Rect {
                    x0: f64::from(rect.x),
                    y0: f64::from(rect.y),
                    x1: f64::from(rect.x + rect.width),
                    y1: f64::from(rect.y + rect.height),
                });
                (self.id.part(index as u64), node)
            })
            .collect();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: None,
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        self.press(part as usize);
        true
    }
}
