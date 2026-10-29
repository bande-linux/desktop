use accesskit::{Action, Affine, Node, NodeId, Role, TreeUpdate};
use unbound_graphics::{
    Color, Item, Palette, Paragraph, Point, Rect, Scene, Scheme, Shape, TextAlign, TextStyle,
    TextSystem,
};

use crate::L10N;
use crate::window::ResizeEdge;

pub const TITLE_BAR_HEIGHT: u32 = 28;
pub(crate) const SHADOW_MARGIN: u32 = 36;
const RESIZE_BAND: f32 = 6.0;
const CORNER_REACH: f32 = 14.0;
const WINDOW_RADIUS: f32 = 10.0;
const CONTROL_DIAMETER: f32 = 12.0;
const CONTROL_PITCH: f32 = 20.0;
const CONTROLS_INSET: f32 = 12.0;
const TITLE_SIZE: f32 = 13.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Control {
    Close,
    Minimize,
    Zoom,
}

const CONTROLS: [Control; 3] = [Control::Close, Control::Minimize, Control::Zoom];

const WINDOW_NODE: NodeId = NodeId(u64::MAX);
const TITLE_BAR_NODE: NodeId = NodeId(u64::MAX - 1);

impl Control {
    fn node(self) -> NodeId {
        NodeId(match self {
            Self::Close => u64::MAX - 2,
            Self::Minimize => u64::MAX - 3,
            Self::Zoom => u64::MAX - 4,
        })
    }

    pub(crate) fn for_node(node: NodeId) -> Option<Self> {
        CONTROLS.into_iter().find(|control| control.node() == node)
    }

    fn label(self) -> String {
        L10N.tr(match self {
            Self::Close => "close",
            Self::Minimize => "minimize",
            Self::Zoom => "zoom",
        })
    }
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
            return CONTROLS
                .into_iter()
                .find(|control| contains(control_area(*control, self.title), local))
                .map_or(Zone::TitleBar, Zone::Control);
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

fn control_centre(control: Control, title: f32) -> Point {
    let index = CONTROLS
        .iter()
        .position(|candidate| *candidate == control)
        .unwrap_or(0);
    Point::new(
        CONTROLS_INSET + CONTROL_DIAMETER / 2.0 + index as f32 * CONTROL_PITCH,
        title / 2.0,
    )
}

fn control_area(control: Control, title: f32) -> Rect {
    let centre = control_centre(control, title);
    Rect::new(centre.x - CONTROL_PITCH / 2.0, 0.0, CONTROL_PITCH, title)
}

fn controls_group(title: f32) -> Rect {
    let first = control_area(Control::Close, title);
    Rect::new(first.x, first.y, CONTROL_PITCH * 3.0, title)
}

#[derive(Default)]
pub(crate) struct Chrome {
    pub(crate) title: String,
    text: Option<TextSystem>,
    title_paragraph: Option<(String, Color, Paragraph)>,
    pub(crate) hovered_group: bool,
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
        bar.set_children(CONTROLS.map(Control::node).to_vec());
        tree.nodes.push((WINDOW_NODE, window));
        tree.nodes.push((TITLE_BAR_NODE, bar));
        for control in CONTROLS {
            let mut button = Node::new(Role::Button);
            button.set_label(control.label());
            button.set_bounds(bounds(control_area(control, frame.title)));
            button.add_action(Action::Click);
            tree.nodes.push((control.node(), button));
        }
        tree
    }

    pub(crate) fn hover(&mut self, frame: &Frame, point: Option<Point>) -> bool {
        let group = point.is_some_and(|point| {
            contains(
                controls_group(frame.title),
                Point::new(point.x - frame.margin, point.y - frame.margin),
            )
        });
        let changed = group != self.hovered_group;
        self.hovered_group = group;
        changed
    }

    fn title(&mut self, color: Color) -> Paragraph {
        if let Some((title, cached_color, paragraph)) = &self.title_paragraph
            && *title == self.title
            && *cached_color == color
        {
            return paragraph.clone();
        }
        let text = self.text.get_or_insert_with(TextSystem::new);
        let paragraph = text.paragraph(
            &self.title,
            TextStyle::new(TITLE_SIZE, color).weight(TextStyle::SEMIBOLD),
            None,
            TextAlign::Start,
        );
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
    ) -> Scene {
        let window = Rect::new(0.0, 0.0, frame.width, frame.height);
        let radius = if frame.margin > 0.0 {
            WINDOW_RADIUS
        } else {
            0.0
        };
        let outline = Shape::rounded(window, radius);
        let mut items = Vec::new();
        if frame.margin > 0.0 {
            items.extend(shadows(outline, active));
        }
        let mut inside = vec![
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
            self.title_bar(frame, palette, scheme, active, &mut inside);
        }
        items.push(if radius > 0.0 {
            Item::Clip {
                shape: outline,
                items: inside,
            }
        } else {
            Item::Translate {
                offset: Point::default(),
                items: inside,
            }
        });
        if frame.margin > 0.0 {
            items.extend(border(outline, scheme));
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
        items: &mut Vec<Item>,
    ) {
        let bar = Rect::new(0.0, 0.0, frame.width, frame.title);
        items.push(Item::Fill {
            shape: Shape::rect(bar),
            paint: palette.surface_elevated.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(0.0, frame.title - 1.0, frame.width, 1.0)),
            paint: palette.separator_color.into(),
        });
        let color = if active {
            palette.label_primary
        } else {
            palette.label_muted
        };
        let paragraph = self.title(color);
        let reserved = controls_group(frame.title);
        let side = reserved.x + reserved.width + 8.0;
        let available = Rect::new(side, 0.0, (frame.width - 2.0 * side).max(0.0), frame.title);
        let x = ((frame.width - paragraph.width()) / 2.0)
            .max(available.x)
            .round();
        let y = ((frame.title - paragraph.height()) / 2.0).round();
        items.push(Item::Clip {
            shape: Shape::rect(available),
            items: vec![Item::Text {
                origin: Point::new(x, y),
                paragraph,
            }],
        });
        let lit = active || self.hovered_group;
        for control in CONTROLS {
            self.control(control, frame.title, scheme, lit, items);
        }
    }

    fn control(
        &self,
        control: Control,
        title: f32,
        scheme: Scheme,
        lit: bool,
        items: &mut Vec<Item>,
    ) {
        let centre = control_centre(control, title);
        let half = CONTROL_DIAMETER / 2.0;
        let disc = Rect::new(
            centre.x - half,
            centre.y - half,
            CONTROL_DIAMETER,
            CONTROL_DIAMETER,
        );
        let (fill, edge, glyph) = control_colors(control, scheme, lit);
        let pressed = self.pressed == Some(control) && self.pressed_inside;
        let fill = if pressed { darker(fill) } else { fill };
        items.push(Item::Fill {
            shape: Shape::rounded(disc, half),
            paint: fill.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(disc.inset(0.25), half - 0.25),
            width: 0.5,
            paint: edge.into(),
        });
        if !(lit && self.hovered_group) {
            return;
        }
        let line = |from: (f32, f32), to: (f32, f32)| Item::Line {
            from: Point::new(centre.x + from.0, centre.y + from.1),
            to: Point::new(centre.x + to.0, centre.y + to.1),
            width: 1.1,
            paint: glyph.with_alpha(200).into(),
        };
        let arm = 2.75;
        match control {
            Control::Close => {
                items.push(line((-arm, -arm), (arm, arm)));
                items.push(line((-arm, arm), (arm, -arm)));
            }
            Control::Minimize => items.push(line((-3.5, 0.0), (3.5, 0.0))),
            Control::Zoom => {
                items.push(line((-3.5, 0.0), (3.5, 0.0)));
                items.push(line((0.0, -3.5), (0.0, 3.5)));
            }
        }
    }
}

fn control_colors(control: Control, scheme: Scheme, lit: bool) -> (Color, Color, Color) {
    if !lit {
        return match scheme {
            Scheme::Light => (
                Color::hex(0xD9DADD),
                Color::hex(0xC3C5CA),
                Color::TRANSPARENT,
            ),
            Scheme::Dark => (
                Color::hex(0x4A4D54),
                Color::hex(0x5A5E66),
                Color::TRANSPARENT,
            ),
        };
    }
    match control {
        Control::Close => (
            Color::hex(0xFF5F57),
            Color::hex(0xE24B41),
            Color::hex(0x7A0F08),
        ),
        Control::Minimize => (
            Color::hex(0xFEBC2E),
            Color::hex(0xE1A116),
            Color::hex(0x8A5300),
        ),
        Control::Zoom => (
            Color::hex(0x28C840),
            Color::hex(0x1AAB29),
            Color::hex(0x0B5E14),
        ),
    }
}

fn darker(color: Color) -> Color {
    let scale = |channel: u8| (u16::from(channel) * 4 / 5) as u8;
    Color {
        r: scale(color.r),
        g: scale(color.g),
        b: scale(color.b),
        a: color.a,
    }
}

fn shadows(outline: Shape, active: bool) -> [Item; 2] {
    let black = Color::hex(0x000000);
    let (blur, offset, alpha) = if active {
        (20.0, 8.0, 80)
    } else {
        (12.0, 4.0, 46)
    };
    [
        Item::Shadow {
            shape: outline,
            offset: Point::new(0.0, offset),
            blur_radius: blur,
            color: black.with_alpha(alpha),
        },
        Item::Shadow {
            shape: outline,
            offset: Point::default(),
            blur_radius: 2.0,
            color: black.with_alpha(28),
        },
    ]
}

fn border(outline: Shape, scheme: Scheme) -> Vec<Item> {
    let outer = Item::Stroke {
        shape: Shape::rounded(outline.rect.inset(-0.5), outline.radius + 0.5),
        width: 1.0,
        paint: Color::hex(0x000000)
            .with_alpha(match scheme {
                Scheme::Light => 30,
                Scheme::Dark => 140,
            })
            .into(),
    };
    match scheme {
        Scheme::Light => vec![outer],
        Scheme::Dark => vec![
            outer,
            Item::Stroke {
                shape: Shape::rounded(outline.rect.inset(0.5), outline.radius - 0.5),
                width: 1.0,
                paint: Color::hex(0xFFFFFF).with_alpha(26).into(),
            },
        ],
    }
}
