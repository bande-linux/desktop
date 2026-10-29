use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_config::DockEdge;
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{Context, Event, Size, Ui, Widget, WidgetId};
use unbound_toolkit::{
    Color, Image, Item, Paint, Palette, Paragraph, Point, Rect, Scene, Shape, Stop, TextAlign,
    TextStyle, TextSystem,
};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-dock.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const THICKNESS: u32 = 64;
pub const ICON_SIZE: u32 = 48;
const TILE_SPACING: f32 = 4.0;
const EDGE_PADDING: f32 = 8.0;
const ICON_RADIUS: f32 = 10.75;
const INDICATOR: f32 = 4.0;
const CLOCK_SIZE: f32 = 13.0;

#[derive(Debug, Clone)]
pub struct DockApp {
    pub app_id: String,
    pub name: String,
    pub icon: Option<Image>,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockAction {
    Open(String),
}

pub type Actions = Rc<RefCell<Vec<DockAction>>>;

fn contains(frame: Rect, point: Point) -> bool {
    point.x >= frame.x
        && point.y >= frame.y
        && point.x < frame.x + frame.width
        && point.y < frame.y + frame.height
}

fn vertical(edge: DockEdge) -> bool {
    matches!(edge, DockEdge::Left | DockEdge::Right)
}

struct Tile {
    id: WidgetId,
    app: DockApp,
    edge: DockEdge,
    actions: Actions,
    pressed: bool,
    initial: Option<Paragraph>,
    frame: Rect,
}

impl Tile {
    fn icon_rect(&self) -> Rect {
        let size = ICON_SIZE as f32;
        Rect::new(
            (self.frame.x + (self.frame.width - size) / 2.0).round(),
            (self.frame.y + (self.frame.height - size) / 2.0).round(),
            size,
            size,
        )
    }

    fn open(&mut self) {
        self.actions
            .borrow_mut()
            .push(DockAction::Open(self.app.app_id.clone()));
    }
}

fn generic_icon(rect: Rect, initial: Option<&Paragraph>, items: &mut Vec<Item>) {
    let shape = Shape::rounded(rect, ICON_RADIUS);
    items.push(Item::Fill {
        shape,
        paint: Paint::LinearGradient {
            start: Point::new(rect.x, rect.y),
            end: Point::new(rect.x, rect.y + rect.height),
            stops: vec![
                Stop {
                    offset: 0.0,
                    color: Color::hex(0x8E9AAF),
                },
                Stop {
                    offset: 1.0,
                    color: Color::hex(0x5C677D),
                },
            ],
        },
    });
    if let Some(initial) = initial {
        items.push(Item::Text {
            origin: Point::new(
                (rect.x + (rect.width - initial.width()) / 2.0).round(),
                (rect.y + (rect.height - initial.height()) / 2.0).round(),
            ),
            paragraph: initial.clone(),
        });
    }
}

impl Widget for Tile {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        if self.app.icon.is_none() {
            let initial = self
                .app
                .name
                .chars()
                .next()
                .map(|character| character.to_uppercase().collect::<String>())
                .unwrap_or_default();
            self.initial = Some(cx.text.paragraph(
                &initial,
                TextStyle::new(24.0, Color::hex(0xFFFFFF)).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            ));
        }
        let side = THICKNESS as f32;
        let length = ICON_SIZE as f32 + TILE_SPACING;
        if vertical(self.edge) {
            Size::new(side, length)
        } else {
            Size::new(length, side)
        }
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let rect = self.icon_rect();
        match &self.app.icon {
            Some(image) => items.push(Item::Image {
                rect,
                image: image.clone(),
            }),
            None => generic_icon(rect, self.initial.as_ref(), items),
        }
        if self.pressed {
            items.push(Item::Fill {
                shape: Shape::rounded(rect, ICON_RADIUS),
                paint: Color::hex(0x000000).with_alpha(64).into(),
            });
        }
        if self.app.running {
            let center = match self.edge {
                DockEdge::Left => Point::new(self.frame.x + 3.0, rect.y + rect.height / 2.0),
                DockEdge::Right => Point::new(
                    self.frame.x + self.frame.width - 3.0,
                    rect.y + rect.height / 2.0,
                ),
                DockEdge::Bottom => Point::new(
                    rect.x + rect.width / 2.0,
                    self.frame.y + self.frame.height - 3.0,
                ),
                DockEdge::Top => Point::new(rect.x + rect.width / 2.0, self.frame.y + 3.0),
            };
            items.push(Item::Fill {
                shape: Shape::rounded(
                    Rect::new(
                        center.x - INDICATOR / 2.0,
                        center.y - INDICATOR / 2.0,
                        INDICATOR,
                        INDICATOR,
                    ),
                    INDICATOR / 2.0,
                ),
                paint: cx.palette.label_primary.into(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Button);
        node.set_label(self.app.name.clone());
        node.add_action(Action::Click);
        node
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => self.pressed = true,
            Event::PointerMove(pointer) if self.pressed => {
                self.pressed = contains(self.frame, pointer.position);
            }
            Event::PointerUp(pointer) if pointer.is_primary() => {
                let inside = self.pressed && contains(self.frame, pointer.position);
                self.pressed = false;
                if inside {
                    self.open();
                }
            }
            _ => return false,
        }
        true
    }

    fn perform(&mut self, _cx: &mut Context, request: &ActionRequest) -> bool {
        if request.action != Action::Click {
            return false;
        }
        self.open();
        true
    }
}

struct Clock {
    id: WidgetId,
    text: String,
    paragraph: Option<Paragraph>,
    frame: Rect,
}

impl Widget for Clock {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let paragraph = cx.text.paragraph(
            &self.text,
            TextStyle::new(CLOCK_SIZE, cx.palette.label_primary).weight(TextStyle::MEDIUM),
            None,
            TextAlign::Start,
        );
        let size = Size::new(paragraph.width().ceil(), paragraph.height().ceil());
        self.paragraph = Some(paragraph);
        size
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

struct Dock {
    id: WidgetId,
    edge: DockEdge,
    children: Vec<Box<dyn Widget>>,
    frame: Rect,
}

impl Widget for Dock {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let along = vertical(self.edge);
        let mut offset = EDGE_PADDING;
        let count = self.children.len();
        for (index, child) in self.children.iter_mut().enumerate() {
            let size = child.measure(cx, Size::new(frame.width, frame.height));
            if index + 1 == count {
                let rect = if along {
                    Rect::new(
                        frame.x + ((frame.width - size.width) / 2.0).round(),
                        frame.y + frame.height - EDGE_PADDING - size.height,
                        size.width,
                        size.height,
                    )
                } else {
                    Rect::new(
                        frame.x + frame.width - EDGE_PADDING - size.width,
                        frame.y + ((frame.height - size.height) / 2.0).round(),
                        size.width,
                        size.height,
                    )
                };
                child.place(cx, rect);
                continue;
            }
            let rect = if along {
                Rect::new(frame.x, frame.y + offset, frame.width, size.height)
            } else {
                Rect::new(frame.x + offset, frame.y, size.width, frame.height)
            };
            offset += if along { size.height } else { size.width };
            child.place(cx, rect);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Fill {
            shape: Shape::rect(self.frame),
            paint: cx.palette.surface_elevated.into(),
        });
        let frame = self.frame;
        let separator = match self.edge {
            DockEdge::Left => Rect::new(frame.x + frame.width - 1.0, frame.y, 1.0, frame.height),
            DockEdge::Right => Rect::new(frame.x, frame.y, 1.0, frame.height),
            DockEdge::Top => Rect::new(frame.x, frame.y + frame.height - 1.0, frame.width, 1.0),
            DockEdge::Bottom => Rect::new(frame.x, frame.y, frame.width, 1.0),
        };
        items.push(Item::Fill {
            shape: Shape::rect(separator),
            paint: cx.palette.separator_color.into(),
        });
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Toolbar);
        node.set_label(L10N.tr("dock"));
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub fn dock_ui(
    edge: DockEdge,
    apps: Vec<DockApp>,
    clock: &str,
    palette: Palette,
    text: TextSystem,
    actions: Actions,
) -> Ui {
    let mut children: Vec<Box<dyn Widget>> = apps
        .into_iter()
        .map(|app| {
            Box::new(Tile {
                id: WidgetId::next(),
                app,
                edge,
                actions: actions.clone(),
                pressed: false,
                initial: None,
                frame: Rect::default(),
            }) as Box<dyn Widget>
        })
        .collect();
    children.push(Box::new(Clock {
        id: WidgetId::next(),
        text: clock.to_owned(),
        paragraph: None,
        frame: Rect::default(),
    }));
    Ui::new(
        Box::new(Dock {
            id: WidgetId::next(),
            edge,
            children,
            frame: Rect::default(),
        }),
        Context::new(text, palette),
    )
}

pub fn dock_size(edge: DockEdge, output: (u32, u32)) -> (u32, u32) {
    if vertical(edge) {
        (THICKNESS, output.1)
    } else {
        (output.0, THICKNESS)
    }
}

pub fn scene(ui: &mut Ui, size: (u32, u32)) -> Scene {
    ui.scene(size.0, size.1)
}

pub fn clock_text(now: &jiff::Zoned) -> String {
    now.strftime("%H:%M").to_string()
}

pub fn merge_apps(
    pinned: &[DockApp],
    running: &[String],
    describe: impl Fn(&str) -> DockApp,
) -> Vec<DockApp> {
    let mut apps: Vec<DockApp> = pinned
        .iter()
        .cloned()
        .map(|mut app| {
            app.running = running.contains(&app.app_id);
            app
        })
        .collect();
    for app_id in running {
        if !apps.iter().any(|app| &app.app_id == app_id) {
            apps.push(DockApp {
                running: true,
                ..describe(app_id)
            });
        }
    }
    apps
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("lib.rs"), include_str!("main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
