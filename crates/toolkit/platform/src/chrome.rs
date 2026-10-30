use accesskit::{Action, Affine, Node, NodeId, Role, TreeUpdate};
use unbound_graphics::{
    Color, Item, Palette, Paragraph, Point, Rect, Scene, Scheme, Shape, TextAlign, TextStyle,
    TextSystem, TitleBar, title_style, window_controls_area,
};

use std::time::Instant;

use unbound_widgets::{Animated, Spring};

use crate::L10N;
use crate::window::ResizeEdge;

pub(crate) use unbound_graphics::RESIZE_MARGIN;
pub use unbound_graphics::TITLE_BAR_HEIGHT;
pub(crate) use unbound_graphics::WindowControl as Control;
const RESIZE_BAND: f32 = RESIZE_MARGIN as f32;
const CORNER_REACH: f32 = unbound_graphics::RESIZE_CORNER;

const WINDOW_NODE: NodeId = NodeId(u64::MAX);
const TITLE_BAR_NODE: NodeId = NodeId(u64::MAX - 1);

fn control_node(control: Control) -> NodeId {
    NodeId(match control {
        Control::Close => u64::MAX - 2,
        Control::Minimize => u64::MAX - 3,
        Control::Zoom => u64::MAX - 4,
    })
}

pub(crate) fn control_for_node(node: NodeId) -> Option<Control> {
    Control::ALL
        .into_iter()
        .find(|control| control_node(*control) == node)
}

fn control_label(control: Control) -> String {
    L10N.tr(match control {
        Control::Close => "close",
        Control::Minimize => "minimize",
        Control::Zoom => "zoom",
    })
}

fn bounds(rect: Rect) -> accesskit::Rect {
    accesskit::Rect {
        x0: f64::from(rect.x),
        y0: f64::from(rect.y),
        x1: f64::from(rect.x + rect.width),
        y1: f64::from(rect.y + rect.height),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Zone {
    Outside,
    Resize(ResizeEdge),
    Control(Control),
    TitleBar,
    Content,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub(crate) margin: f32,
    pub(crate) title: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl Frame {
    fn window(&self) -> Rect {
        Rect::new(self.margin, self.margin, self.width, self.height)
    }

    pub(crate) fn content_origin(&self) -> Point {
        Point::new(self.margin, self.margin + self.title)
    }

    pub(crate) fn zone_at(&self, point: Point) -> Zone {
        let window = self.window();
        let inside = point.x >= window.x
            && point.y >= window.y
            && point.x < window.x + window.width
            && point.y < window.y + window.height;
        if inside {
            if point.y >= window.y + self.title {
                return Zone::Content;
            }
            let local = Point::new(point.x - window.x, point.y - window.y);
            return Control::at(local, self.title).map_or(Zone::TitleBar, Zone::Control);
        }
        if self.margin == 0.0 || !contains(window.inset(-RESIZE_BAND), point) {
            return Zone::Outside;
        }
        let horizontal = if point.x < window.x + CORNER_REACH {
            -1
        } else if point.x >= window.x + window.width - CORNER_REACH {
            1
        } else {
            0
        };
        let vertical = if point.y < window.y + CORNER_REACH {
            -1
        } else if point.y >= window.y + window.height - CORNER_REACH {
            1
        } else {
            0
        };
        Zone::Resize(match (vertical, horizontal) {
            (-1, -1) => ResizeEdge::TopLeft,
            (-1, 1) => ResizeEdge::TopRight,
            (1, -1) => ResizeEdge::BottomLeft,
            (1, 1) => ResizeEdge::BottomRight,
            (-1, _) => ResizeEdge::Top,
            (1, _) => ResizeEdge::Bottom,
            (_, -1) => ResizeEdge::Left,
            _ => ResizeEdge::Right,
        })
    }

    pub(crate) fn input_area(&self) -> Rect {
        self.window().inset(-RESIZE_BAND)
    }
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

#[derive(Default)]
pub(crate) struct Chrome {
    pub(crate) title: String,
    text: Option<TextSystem>,
    title_paragraph: Option<(String, Color, Paragraph)>,
    hovered_group: bool,
    glyphs: Animated,
    pub(crate) animating: bool,
    pub(crate) pressed: Option<Control>,
    pub(crate) pressed_inside: bool,
    pub(crate) pointer_in_content: bool,
    pub(crate) content_buttons: usize,
}

impl Chrome {
    pub(crate) fn new(title: String) -> Self {
        Self {
            title,
            ..Self::default()
        }
    }

    pub(crate) fn wrap_tree(&self, frame: &Frame, mut tree: TreeUpdate) -> TreeUpdate {
        let Some(info) = tree.tree.as_mut() else {
            return tree;
        };
        let content = std::mem::replace(&mut info.root, WINDOW_NODE);
        if let Some((_, node)) = tree.nodes.iter_mut().find(|(id, _)| *id == content) {
            node.set_transform(Affine::translate((0.0, f64::from(frame.title))));
        }
        let mut window = Node::new(Role::Window);
        window.set_label(self.title.clone());
        window.set_bounds(bounds(Rect::new(0.0, 0.0, frame.width, frame.height)));
        window.set_children(vec![TITLE_BAR_NODE, content]);
        let mut bar = Node::new(Role::TitleBar);
        bar.set_label(self.title.clone());
        bar.set_bounds(bounds(Rect::new(0.0, 0.0, frame.width, frame.title)));
        bar.set_children(Control::ALL.map(control_node).to_vec());
        tree.nodes.push((WINDOW_NODE, window));
        tree.nodes.push((TITLE_BAR_NODE, bar));
        for control in Control::ALL {
            let mut button = Node::new(Role::Button);
            button.set_label(control_label(control));
            button.set_bounds(bounds(control.area(frame.title)));
            button.add_action(Action::Click);
            tree.nodes.push((control_node(control), button));
        }
        tree
    }

    pub(crate) fn hover(
        &mut self,
        frame: &Frame,
        point: Option<Point>,
        now: Instant,
        reduce_motion: bool,
    ) -> bool {
        let group = point.is_some_and(|point| {
            contains(
                window_controls_area(frame.title),
                Point::new(point.x - frame.margin, point.y - frame.margin),
            )
        });
        let changed = group != self.hovered_group;
        self.hovered_group = group;
        let shown = if group { 1.0 } else { 0.0 };
        self.glyphs
            .animate_at(now, reduce_motion, shown, Spring::FADE);
        changed
    }

    fn title(&mut self, style: TextStyle) -> Paragraph {
        if let Some((title, cached_color, paragraph)) = &self.title_paragraph
            && *title == self.title
            && *cached_color == style.color
        {
            return paragraph.clone();
        }
        let color = style.color;
        let text = self.text.get_or_insert_with(TextSystem::new);
        let paragraph = text.paragraph(&self.title, style, None, TextAlign::Start);
        self.title_paragraph = Some((self.title.clone(), color, paragraph.clone()));
        paragraph
    }

    pub(crate) fn frame_scene(
        &mut self,
        frame: &Frame,
        content: &Scene,
        palette: &Palette,
        scheme: Scheme,
        active: bool,
        now: Instant,
    ) -> Scene {
        let mut items = vec![
            Item::Fill {
                shape: Shape::rect(Rect::new(
                    0.0,
                    frame.title,
                    frame.width,
                    frame.height - frame.title,
                )),
                paint: content.background.into(),
            },
            Item::Translate {
                offset: Point::new(0.0, frame.title),
                items: content.items.clone(),
            },
        ];
        if frame.title > 0.0 {
            self.title_bar(frame, palette, scheme, active, now, &mut items);
        }
        Scene {
            width: (frame.width + 2.0 * frame.margin) as u32,
            height: (frame.height + 2.0 * frame.margin) as u32,
            background: Color::TRANSPARENT,
            items: vec![Item::Translate {
                offset: Point::new(frame.margin, frame.margin),
                items,
            }],
        }
    }

    fn title_bar(
        &mut self,
        frame: &Frame,
        palette: &Palette,
        scheme: Scheme,
        active: bool,
        now: Instant,
        items: &mut Vec<Item>,
    ) {
        let title = self.title(title_style(palette, active));
        let (shown, moving) = self.glyphs.value_at(now);
        self.animating = moving;
        items.extend(
            TitleBar {
                width: frame.width,
                height: frame.title,
                title,
                palette,
                scheme,
                active,
                glyphs: shown,
                pressed: self.pressed.filter(|_| self.pressed_inside),
            }
            .items(),
        );
    }
}
