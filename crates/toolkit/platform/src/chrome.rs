use accesskit::{Action, Affine, Node, NodeId, Role, TreeUpdate};
use bande_graphics::{
    Color, Cut, Item, Palette, Paragraph, Point, Radius, Rect, ResizeEdge, Scene, Shape, TextAlign,
    TextStyle, TextSystem, TitleBar, resize_zone, shortened, title_style, window_controls_area,
};

use std::time::Instant;

use bande_widgets::{Animated, Spring};

use crate::L10N;

pub(crate) use bande_graphics::RESIZE_MARGIN;
pub use bande_graphics::WINDOW_HEADER_HEIGHT;
pub const SHEET_INSET: u32 = 6;
pub(crate) use bande_graphics::WindowControl as Control;
const TITLE_TRAILING_GAP: f32 = 12.0;

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

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameRegions {
    pub sidebar: f32,
    pub toolbar: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Header {
    pub title_x: f32,
    pub items: Vec<Rect>,
    pub tabs: Vec<Rect>,
}

impl Header {
    fn title_area(&self, width: f32) -> Rect {
        let end = self
            .items
            .iter()
            .chain(&self.tabs)
            .map(|item| item.x)
            .filter(|x| *x > self.title_x)
            .fold(width, f32::min);
        Rect::new(
            self.title_x,
            0.0,
            (end - self.title_x - TITLE_TRAILING_GAP).max(0.0),
            WINDOW_HEADER_HEIGHT as f32,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Frame {
    pub(crate) margin: f32,
    pub(crate) resize_radius: Option<f32>,
    pub(crate) title: f32,
    pub(crate) inset: f32,
    pub(crate) regions: Option<FrameRegions>,
    pub(crate) header: bool,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl Frame {
    pub(crate) fn window(&self) -> Rect {
        Rect::new(self.margin, self.margin, self.width, self.height)
    }

    fn sheet(&self) -> Rect {
        let regions = self.regions.unwrap_or_default();
        let left = regions.sidebar.max(self.inset);
        let top = self.title + regions.toolbar;
        Rect::new(
            left,
            top,
            (self.width - left - self.inset).max(0.0),
            (self.height - top - self.inset).max(0.0),
        )
    }

    fn sheet_radius(&self) -> f32 {
        if self.inset > 0.0 {
            Radius::concentric(Radius::XL, self.inset)
        } else {
            0.0
        }
    }

    fn content(&self) -> Rect {
        match self.regions {
            _ if self.header => Rect::new(0.0, 0.0, self.width, self.height),
            Some(_) => Rect::new(0.0, self.title, self.width, self.height - self.title),
            None => self.sheet(),
        }
    }

    pub(crate) fn content_origin(&self) -> Point {
        let content = self.content();
        Point::new(self.margin + content.x, self.margin + content.y)
    }

    pub(crate) fn content_size(&self) -> (u32, u32) {
        let content = self.content();
        (
            content.width.max(0.0).round() as u32,
            content.height.max(0.0).round() as u32,
        )
    }

    pub(crate) fn chrome_size(&self) -> (u32, u32) {
        let (width, height) = self.content_size();
        (
            (self.width.round() as u32).saturating_sub(width),
            (self.height.round() as u32).saturating_sub(height),
        )
    }

    pub(crate) fn zone_at(&self, point: Point) -> Zone {
        let window = self.window();
        if let Some(edge) = self
            .resize_radius
            .and_then(|radius| resize_zone(window, radius, self.margin, point))
        {
            return Zone::Resize(edge);
        }
        if window.contains(point) {
            let local = Point::new(point.x - window.x, point.y - window.y);
            if local.y < self.title {
                return Control::at(local, self.title).map_or(Zone::TitleBar, Zone::Control);
            }
            return if self.content().contains(local) {
                Zone::Content
            } else {
                Zone::TitleBar
            };
        }
        Zone::Outside
    }

    pub(crate) fn input_area(&self) -> Rect {
        self.window().inset(-self.margin)
    }
}

#[derive(Default)]
pub(crate) struct Chrome {
    pub(crate) title: String,
    pub(crate) titled: bool,
    pub(crate) fixed: bool,
    text: Option<TextSystem>,
    title_paragraph: Option<(String, Color, Paragraph)>,
    hovered: Option<Option<Control>>,
    hover: Animated,
    close_hover: Animated,
    pub(crate) alternate: bool,
    pub(crate) regions: Option<FrameRegions>,
    pub(crate) header: Option<Header>,
    pub(crate) animating: bool,
    pub(crate) pressed: Option<Control>,
    pub(crate) pressed_inside: bool,
    pub(crate) pointer_in_content: bool,
    pub(crate) content_buttons: usize,
    pub(crate) tab_press: Option<Point>,
    pub(crate) zoom_rest: Option<Instant>,
}

impl Chrome {
    pub(crate) fn new(title: String, titled: bool, fixed: bool) -> Self {
        Self {
            title,
            titled,
            fixed,
            ..Self::default()
        }
    }

    pub(crate) fn zone_at(&self, frame: &Frame, point: Point) -> Zone {
        let zone = frame.zone_at(point);
        let local = Point::new(point.x - frame.margin, point.y - frame.margin);
        match &self.header {
            Some(header)
                if zone == Zone::TitleBar
                    && header
                        .items
                        .iter()
                        .chain(&header.tabs)
                        .any(|item| item.contains(local)) =>
            {
                Zone::Content
            }
            _ => zone,
        }
    }

    pub(crate) fn tab_at(&self, frame: &Frame, point: Point) -> bool {
        let local = Point::new(point.x - frame.margin, point.y - frame.margin);
        frame.zone_at(point) == Zone::TitleBar
            && self
                .header
                .as_ref()
                .is_some_and(|header| header.tabs.iter().any(|tab| tab.contains(local)))
    }

    pub(crate) fn wrap_tree(&self, frame: &Frame, mut tree: TreeUpdate) -> TreeUpdate {
        let Some(info) = tree.tree.as_mut() else {
            return tree;
        };
        let content = std::mem::replace(&mut info.root, WINDOW_NODE);
        if let Some((_, node)) = tree.nodes.iter_mut().find(|(id, _)| *id == content) {
            let content = frame.content();
            node.set_transform(Affine::translate((
                f64::from(content.x),
                f64::from(content.y),
            )));
        }
        let mut window = Node::new(match self.titled {
            true => Role::Window,
            false => Role::Dialog,
        });
        window.set_label(self.title.clone());
        window.set_bounds(bounds(Rect::new(0.0, 0.0, frame.width, frame.height)));
        if !self.titled {
            window.set_children(vec![content]);
            tree.nodes.push((WINDOW_NODE, window));
            return tree;
        }
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
        let hovered = point
            .map(|point| Point::new(point.x - frame.margin, point.y - frame.margin))
            .filter(|local| window_controls_area(frame.title).contains(*local))
            .map(|local| Control::at(local, frame.title));
        if hovered == self.hovered {
            return false;
        }
        self.hovered = hovered;
        self.zoom_rest = (hovered == Some(Some(Control::Zoom))).then_some(now);
        let level = |on: bool| if on { 1.0 } else { 0.0 };
        self.hover
            .animate_at(now, reduce_motion, level(hovered.is_some()), Spring::FADE);
        self.close_hover.animate_at(
            now,
            reduce_motion,
            level(hovered == Some(Some(Control::Close))),
            Spring::FADE,
        );
        true
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
        content: Scene,
        palette: &Palette,
        look: Look,
        now: Instant,
    ) -> Scene {
        let mut items = vec![Item::Fill {
            shape: Shape::rect(Rect::new(0.0, 0.0, frame.width, frame.height)),
            paint: palette.chassis.into(),
        }];
        if frame.title > 0.0 {
            self.title_bar(frame, palette, look, now, &mut items);
        }
        let sheet = frame.sheet();
        let radius = frame.sheet_radius();
        items.push(Item::Fill {
            shape: Shape::rounded(sheet, radius),
            paint: content.background.into(),
        });
        let placed = frame.content();
        let local = Rect::new(
            sheet.x - placed.x,
            sheet.y - placed.y,
            sheet.width,
            sheet.height,
        );
        let drawn = match frame.regions {
            _ if frame.header => content.items,
            Some(_) => content.items,
            None => vec![Item::Clip {
                shape: Shape::rounded(local, radius),
                items: content.items,
            }],
        };
        items.push(Item::Translate {
            offset: Point::new(placed.x, placed.y),
            items: drawn,
        });
        if frame.inset > 0.0 {
            items.extend(Item::rim(Shape::rounded(sheet, radius), palette));
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
        look: Look,
        now: Instant,
        items: &mut Vec<Item>,
    ) {
        let style = title_style(palette, look.active);
        let title = self.title(style);
        let (title, header_title) = match &self.header {
            Some(header) => {
                let text = self.text.get_or_insert_with(TextSystem::new);
                let empty = text.paragraph("", style, None, TextAlign::Start);
                let area = header.title_area(frame.width);
                let title = match title.width() > area.width {
                    true => {
                        let shown = shortened(&self.title, Cut::End, |candidate| {
                            text.paragraph(candidate, style, None, TextAlign::Start)
                                .width()
                                <= area.width
                        });
                        text.paragraph(&shown, style, None, TextAlign::Start)
                    }
                    false => title,
                };
                (empty, Some((area, title)))
            }
            None => (title, None),
        };
        let (hover, hovering) = self.hover.value_at(now);
        let (close_hover, closing) = self.close_hover.value_at(now);
        self.animating = hovering || closing;
        items.extend(
            TitleBar {
                width: frame.width,
                height: frame.title,
                title,
                palette,
                active: look.active,
                hover,
                close_hover,
                pressed: self.pressed.filter(|_| self.pressed_inside),
                zoomed: look.zoomed,
                alternate: self.alternate && look.active,
            }
            .items(),
        );
        if let Some((area, paragraph)) = header_title {
            items.push(Item::Clip {
                shape: Shape::rect(area),
                items: vec![Item::Text {
                    origin: Point::new(area.x, ((frame.title - paragraph.height()) / 2.0).round()),
                    paragraph,
                }],
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) active: bool,
    pub(crate) zoomed: bool,
}
