use std::cell::RefCell;
use std::rc::Rc;

use accesskit::{Action, ActionRequest, Node, Role};
use unbound_graphics::Path;
use unbound_toolkit::ui::{Context, Event, Key, Parts, Size, Widget, WidgetId, focus_ring_for};
use unbound_toolkit::{
    Color, CpuRenderer, Image, Item, Palette, Paragraph, Point, Radius, Rect, Scene, Scheme, Shape,
    Snapshot, TextAlign, TextStyle, TextSystem,
};

use crate::L10N;

const INK: Color = Color::hex(0xff3b30);
const HIGHLIGHT: Color = Color::hex(0xffd60a).with_alpha(0x66);
const LINE: f32 = 0.004;
const TEXT: f32 = 0.03;
const SMALLEST: f32 = 4.0;
pub const TOOLBAR_HEIGHT: f32 = 44.0;
const SEGMENT_PADDING: f32 = 10.0;
const SEGMENT_HEIGHT: f32 = 28.0;
const SEGMENT_GAP: f32 = 4.0;
const TOOLBAR_PADDING: f32 = 8.0;
const TEXT_SIZE: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Crop,
    Arrow,
    Rectangle,
    Text,
    Highlight,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Crop(Rect),
    Arrow(Point, Point),
    Rectangle(Rect),
    Highlight(Rect),
    Text(Point, String),
}

fn spanned(from: Point, to: Point) -> Rect {
    Rect::new(
        from.x.min(to.x),
        from.y.min(to.y),
        (from.x - to.x).abs(),
        (from.y - to.y).abs(),
    )
}

pub struct Markup {
    pub width: f32,
    pub height: f32,
    pub tool: Tool,
    edits: Vec<Edit>,
    drag: Option<(Point, Point)>,
    typing: Option<(Point, String)>,
}

impl Markup {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: width as f32,
            height: height as f32,
            tool: Tool::Arrow,
            edits: Vec::new(),
            drag: None,
            typing: None,
        }
    }

    fn unit(&self) -> f32 {
        self.width.max(self.height)
    }

    pub fn crop(&self) -> Rect {
        self.edits
            .iter()
            .rev()
            .find_map(|edit| match edit {
                Edit::Crop(rect) => Some(*rect),
                _ => None,
            })
            .unwrap_or(Rect::new(0.0, 0.0, self.width, self.height))
    }

    pub fn is_typing(&self) -> bool {
        self.typing.is_some()
    }

    fn clamp(&self, point: Point) -> Point {
        Point::new(
            point.x.clamp(0.0, self.width),
            point.y.clamp(0.0, self.height),
        )
    }

    pub fn press(&mut self, at: Point) {
        self.commit_text();
        let at = self.clamp(at);
        match self.tool {
            Tool::Text => self.typing = Some((at, String::new())),
            _ => self.drag = Some((at, at)),
        }
    }

    pub fn drag_to(&mut self, at: Point) {
        let at = self.clamp(at);
        if let Some((_, to)) = &mut self.drag {
            *to = at;
        }
    }

    pub fn release(&mut self) {
        let Some((from, to)) = self.drag.take() else {
            return;
        };
        let rect = spanned(from, to);
        let big_enough = rect.width >= SMALLEST || rect.height >= SMALLEST;
        let edit = match self.tool {
            Tool::Crop if rect.width >= SMALLEST && rect.height >= SMALLEST => Edit::Crop(rect),
            Tool::Arrow if big_enough => Edit::Arrow(from, to),
            Tool::Rectangle if big_enough => Edit::Rectangle(rect),
            Tool::Highlight if big_enough => Edit::Highlight(rect),
            _ => return,
        };
        self.edits.push(edit);
    }

    pub fn type_text(&mut self, text: &str) {
        if let Some((_, typed)) = &mut self.typing {
            typed.push_str(text);
        }
    }

    pub fn key(&mut self, key: Key) -> bool {
        let Some((_, typed)) = &mut self.typing else {
            return false;
        };
        match key {
            Key::Backspace => {
                typed.pop();
            }
            Key::Enter | Key::Escape => self.commit_text(),
            Key::Space => typed.push(' '),
            Key::Character(character) => typed.push(character),
            _ => return false,
        }
        true
    }

    fn commit_text(&mut self) {
        if let Some((at, typed)) = self.typing.take()
            && !typed.trim().is_empty()
        {
            self.edits.push(Edit::Text(at, typed));
        }
    }

    pub fn undo(&mut self) -> bool {
        if self.typing.take().is_some() {
            return true;
        }
        self.edits.pop().is_some()
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    fn arrow(&self, from: Point, to: Point, items: &mut Vec<Item>) {
        let width = self.unit() * LINE;
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let length = dx.hypot(dy).max(1.0);
        let (ux, uy) = (dx / length, dy / length);
        let head = (width * 5.0).min(length * 0.6);
        let base = Point::new(to.x - ux * head, to.y - uy * head);
        items.push(Item::Line {
            from,
            to: base,
            width,
            paint: INK.into(),
        });
        let spread = head * 0.55;
        let path = Path::new()
            .move_to(to)
            .line_to(Point::new(base.x - uy * spread, base.y + ux * spread))
            .line_to(Point::new(base.x + uy * spread, base.y - ux * spread))
            .close();
        items.push(Item::FillPath {
            path,
            paint: INK.into(),
        });
    }

    fn text_paragraph(&self, text: &mut TextSystem, typed: &str) -> Paragraph {
        let style = TextStyle::new((self.unit() * TEXT).max(12.0), INK).weight(TextStyle::SEMIBOLD);
        text.paragraph(typed, style, None, TextAlign::Start)
    }

    fn edit_items(&self, edit: &Edit, text: &mut TextSystem, items: &mut Vec<Item>) {
        let width = self.unit() * LINE;
        match edit {
            Edit::Crop(_) => {}
            Edit::Arrow(from, to) => self.arrow(*from, *to, items),
            Edit::Rectangle(rect) => items.push(Item::Stroke {
                shape: Shape::rounded(*rect, width),
                width,
                paint: INK.into(),
            }),
            Edit::Highlight(rect) => items.push(Item::Fill {
                shape: Shape::rect(*rect),
                paint: HIGHLIGHT.into(),
            }),
            Edit::Text(at, typed) => items.push(Item::Text {
                origin: *at,
                paragraph: self.text_paragraph(text, typed),
            }),
        }
    }

    pub fn items(&self, text: &mut TextSystem, editing: bool) -> Vec<Item> {
        let mut items = Vec::new();
        for edit in &self.edits {
            self.edit_items(edit, text, &mut items);
        }
        if !editing {
            return items;
        }
        if let Some((from, to)) = self.drag {
            let rect = spanned(from, to);
            let pending = match self.tool {
                Tool::Crop => {
                    items.push(Item::Stroke {
                        shape: Shape::rect(rect),
                        width: (self.unit() * 0.002).max(1.0),
                        paint: Palette::for_scheme(Scheme::Light).overlay.into(),
                    });
                    None
                }
                Tool::Arrow => Some(Edit::Arrow(from, to)),
                Tool::Rectangle => Some(Edit::Rectangle(rect)),
                Tool::Highlight => Some(Edit::Highlight(rect)),
                Tool::Text => None,
            };
            if let Some(edit) = pending {
                self.edit_items(&edit, text, &mut items);
            }
        }
        if let Some((at, typed)) = &self.typing {
            let paragraph = self.text_paragraph(text, &format!("{typed}|"));
            items.push(Item::Text {
                origin: *at,
                paragraph,
            });
        }
        items
    }

    pub fn export(&self, image: &Image, text: &mut TextSystem) -> Option<Snapshot> {
        let crop = self.crop();
        let mut items = vec![Item::Image {
            rect: Rect::new(0.0, 0.0, self.width, self.height),
            image: image.clone(),
        }];
        items.extend(self.items(text, false));
        let scene = Scene {
            width: crop.width.round().max(1.0) as u32,
            height: crop.height.round().max(1.0) as u32,
            background: Color::TRANSPARENT,
            items: vec![Item::Translate {
                offset: Point::new(-crop.x, -crop.y),
                items,
            }],
        };
        CpuRenderer::new().render(&scene).ok()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Tool(Tool),
    Undo,
    Delete,
    Copy,
    Done,
}

const COMMANDS: [Command; 9] = [
    Command::Tool(Tool::Crop),
    Command::Tool(Tool::Arrow),
    Command::Tool(Tool::Rectangle),
    Command::Tool(Tool::Text),
    Command::Tool(Tool::Highlight),
    Command::Undo,
    Command::Delete,
    Command::Copy,
    Command::Done,
];

fn title(command: Command) -> String {
    L10N.tr(match command {
        Command::Tool(Tool::Crop) => "crop",
        Command::Tool(Tool::Arrow) => "arrow",
        Command::Tool(Tool::Rectangle) => "rectangle",
        Command::Tool(Tool::Text) => "text",
        Command::Tool(Tool::Highlight) => "highlight",
        Command::Undo => "undo",
        Command::Delete => "delete",
        Command::Copy => "copy",
        Command::Done => "done",
    })
}

pub type Commanded = Rc<RefCell<Vec<Command>>>;

pub struct Toolbar {
    id: WidgetId,
    tool: Tool,
    cursor: usize,
    pressed: Option<usize>,
    labels: Vec<Paragraph>,
    frames: Vec<Rect>,
    commanded: Commanded,
    frame: Rect,
}

impl Toolbar {
    pub fn new(tool: Tool) -> (Self, Commanded) {
        let commanded = Commanded::default();
        (
            Self {
                id: WidgetId::next(),
                tool,
                cursor: 1,
                pressed: None,
                labels: Vec::new(),
                frames: Vec::new(),
                commanded: Rc::clone(&commanded),
                frame: Rect::default(),
            },
            commanded,
        )
    }

    fn command(&mut self, index: usize) {
        let command = COMMANDS[index];
        if let Command::Tool(tool) = command {
            self.tool = tool;
        }
        self.commanded.borrow_mut().push(command);
    }

    fn segment_at(&self, point: Point) -> Option<usize> {
        self.frames.iter().position(|frame| {
            point.x >= frame.x
                && point.y >= frame.y
                && point.x < frame.x + frame.width
                && point.y < frame.y + frame.height
        })
    }
}

impl Widget for Toolbar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        self.labels = COMMANDS
            .iter()
            .map(|command| {
                let color = match command {
                    Command::Done => cx.palette.on_accent,
                    _ => cx.palette.ink,
                };
                cx.text.paragraph(
                    &title(*command),
                    TextStyle::new(TEXT_SIZE, color),
                    None,
                    TextAlign::Start,
                )
            })
            .collect();
        Size::new(available.width, TOOLBAR_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let top = frame.y + (TOOLBAR_HEIGHT - SEGMENT_HEIGHT) / 2.0;
        let widths: Vec<f32> = self
            .labels
            .iter()
            .map(|label| label.width() + 2.0 * SEGMENT_PADDING)
            .collect();
        let trailing: f32 = widths[6..].iter().sum::<f32>() + SEGMENT_GAP * 2.0;
        let mut x = frame.x + TOOLBAR_PADDING;
        let mut frames = Vec::new();
        for (index, width) in widths.iter().enumerate() {
            if index == 5 {
                x += SEGMENT_GAP * 3.0;
            }
            if index == 6 {
                x = (frame.x + frame.width - TOOLBAR_PADDING - trailing).max(x);
            }
            frames.push(Rect::new(x, top, *width, SEGMENT_HEIGHT));
            x += width + SEGMENT_GAP;
        }
        self.frames = frames;
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Fill {
            shape: Shape::rect(self.frame),
            paint: cx.palette.overlay.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                self.frame.x,
                self.frame.y + self.frame.height - 1.0,
                self.frame.width,
                1.0,
            )),
            paint: cx.palette.hairline.into(),
        });
        let focused = cx.is_focused(self.id) && cx.keyboard_navigation;
        for (index, (frame, label)) in self.frames.iter().zip(&self.labels).enumerate() {
            let command = COMMANDS[index];
            let chosen = command == Command::Tool(self.tool);
            let fill = match (command, chosen || self.pressed == Some(index)) {
                (Command::Done, _) => Some(cx.palette.accent),
                (_, true) => Some(cx.palette.well),
                _ => None,
            };
            if let Some(fill) = fill {
                items.push(Item::Fill {
                    shape: Shape::rounded(*frame, Radius::S),
                    paint: fill.into(),
                });
            }
            items.push(Item::Text {
                origin: Point::new(
                    frame.x + SEGMENT_PADDING,
                    (frame.y + (frame.height - label.height()) / 2.0).round(),
                ),
                paragraph: label.clone(),
            });
            if focused && index == self.cursor {
                focus_ring_for(&cx.palette, Shape::rounded(*frame, Radius::S), items);
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Toolbar);
        node.set_label(L10N.tr("markup"));
        node
    }

    fn parts(&self, cx: &Context) -> Parts {
        let nodes: Vec<_> = COMMANDS
            .iter()
            .zip(&self.frames)
            .enumerate()
            .map(|(index, (command, frame))| {
                let mut node = match command {
                    Command::Tool(tool) => {
                        let mut node = Node::new(Role::RadioButton);
                        node.set_toggled(match *tool == self.tool {
                            true => accesskit::Toggled::True,
                            false => accesskit::Toggled::False,
                        });
                        node
                    }
                    _ => Node::new(Role::Button),
                };
                node.set_label(title(*command));
                node.add_action(Action::Click);
                node.set_bounds(accesskit::Rect {
                    x0: f64::from(frame.x),
                    y0: f64::from(frame.y),
                    x1: f64::from(frame.x + frame.width),
                    y1: f64::from(frame.y + frame.height),
                });
                (self.id.part(index as u64), node)
            })
            .collect();
        Parts {
            children: nodes.iter().map(|(id, _)| *id).collect(),
            nodes,
            focus: cx
                .is_focused(self.id)
                .then(|| self.id.part(self.cursor as u64)),
        }
    }

    fn perform_part(&mut self, _cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        let index = part as usize;
        if index >= COMMANDS.len() {
            return false;
        }
        match request.action {
            Action::Click => self.command(index),
            Action::Focus => self.cursor = index,
            _ => return false,
        }
        true
    }

    fn handle(&mut self, _cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                self.pressed = self.segment_at(pointer.position);
                self.pressed.is_some()
            }
            Event::PointerUp(pointer) => {
                let released = self.segment_at(pointer.position);
                if let Some(index) = self.pressed.take().filter(|index| Some(*index) == released) {
                    self.cursor = index;
                    self.command(index);
                }
                true
            }
            Event::Key { key: Key::Left, .. } => {
                self.cursor = self.cursor.saturating_sub(1);
                true
            }
            Event::Key {
                key: Key::Right, ..
            } => {
                self.cursor = (self.cursor + 1).min(COMMANDS.len() - 1);
                true
            }
            Event::Key {
                key: Key::Space | Key::Enter,
                ..
            } => {
                self.command(self.cursor);
                true
            }
            _ => false,
        }
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        cx.keyboard_navigation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(markup: &mut Markup, from: (f32, f32), to: (f32, f32)) {
        markup.press(Point::new(from.0, from.1));
        markup.drag_to(Point::new(to.0, to.1));
        markup.release();
    }

    #[test]
    fn each_tool_leaves_its_mark_and_undo_takes_the_last_back() {
        let mut markup = Markup::new(400, 300);
        markup.tool = Tool::Rectangle;
        drag(&mut markup, (10.0, 10.0), (60.0, 40.0));
        markup.tool = Tool::Arrow;
        drag(&mut markup, (100.0, 100.0), (150.0, 120.0));
        markup.tool = Tool::Text;
        markup.press(Point::new(20.0, 200.0));
        markup.type_text("Hi");
        markup.key(Key::Character('!'));
        markup.key(Key::Enter);

        assert_eq!(
            markup.edits(),
            &[
                Edit::Rectangle(Rect::new(10.0, 10.0, 50.0, 30.0)),
                Edit::Arrow(Point::new(100.0, 100.0), Point::new(150.0, 120.0)),
                Edit::Text(Point::new(20.0, 200.0), "Hi!".to_owned()),
            ]
        );
        assert!(markup.undo());
        assert_eq!(markup.edits().len(), 2);
    }

    #[test]
    fn a_crop_keeps_inside_the_picture_and_the_last_one_counts() {
        let mut markup = Markup::new(400, 300);
        markup.tool = Tool::Crop;
        drag(&mut markup, (50.0, 50.0), (500.0, 250.0));
        drag(&mut markup, (10.0, 20.0), (110.0, 70.0));

        assert_eq!(markup.crop(), Rect::new(10.0, 20.0, 100.0, 50.0));
        markup.undo();
        assert_eq!(markup.crop(), Rect::new(50.0, 50.0, 350.0, 200.0));
    }

    #[test]
    fn a_stray_click_adds_nothing() {
        let mut markup = Markup::new(400, 300);
        drag(&mut markup, (10.0, 10.0), (11.0, 11.0));

        assert!(markup.edits().is_empty());
    }

    #[test]
    fn the_export_is_the_cropped_picture_with_its_marks() {
        let mut markup = Markup::new(40, 30);
        let image = Image::from_rgba(40, 30, [0, 0, 0xff, 0xff].repeat(40 * 30)).expect("image");
        markup.tool = Tool::Highlight;
        drag(&mut markup, (0.0, 0.0), (40.0, 30.0));
        markup.tool = Tool::Crop;
        drag(&mut markup, (10.0, 10.0), (30.0, 20.0));

        let exported = markup
            .export(&image, &mut TextSystem::new())
            .expect("exported");

        assert_eq!((exported.width, exported.height), (20, 10));
        let pixel = &exported.rgba[..4];
        assert!(pixel[0] > 0x40 && pixel[2] < 0xff, "{pixel:?}");
    }
}
