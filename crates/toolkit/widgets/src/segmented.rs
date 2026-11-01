use accesskit::{Action, ActionRequest, Node, Role, Toggled};
use bande_graphics::{
    Color, ControlSize, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextStyle,
};

use super::control::Symbol;
use super::{Animated, Context, Event, Key, Parts, Size, Spring, Widget, WidgetId};

pub const SEGMENTED_HEIGHT: f32 = ControlSize::Regular.height();
const PADDING: f32 = 14.0;
const INSET: f32 = 2.0;
const FONT_SIZE: f32 = 13.0;
const CHIP_SHADOW: f32 = 0.14;
const SYMBOL_SEGMENT: f32 = 32.0;
const SYMBOL_SIZE: f32 = 16.0;
const DISABLED_ALPHA: f32 = 0.45;

struct Segment {
    title: String,
    plain: Option<Paragraph>,
    selected: Option<Paragraph>,
    symbol: Option<(Symbol, Symbol)>,
    width: f32,
}

pub struct SegmentedControl {
    id: WidgetId,
    label: String,
    segments: Vec<Segment>,
    selected: Option<usize>,
    on_select: Box<dyn FnMut(usize)>,
    pill: Option<(Animated, Animated)>,
    enabled: bool,
    fill: bool,
    frame: Rect,
}

impl SegmentedControl {
    pub fn new(
        label: &str,
        choices: &[impl AsRef<str>],
        selected: Option<usize>,
        on_select: impl FnMut(usize) + 'static,
    ) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            segments: choices
                .iter()
                .map(|title| Segment {
                    title: title.as_ref().to_owned(),
                    plain: None,
                    selected: None,
                    symbol: None,
                    width: 0.0,
                })
                .collect(),
            selected,
            on_select: Box::new(on_select),
            pill: None,
            enabled: true,
            fill: false,
            frame: Rect::default(),
        }
    }

    pub fn symbols(
        label: &str,
        choices: Vec<(Image, String)>,
        selected: Option<usize>,
        on_select: impl FnMut(usize) + 'static,
    ) -> Self {
        let titles: Vec<&str> = choices.iter().map(|(_, title)| title.as_str()).collect();
        let mut control = Self::new(label, &titles, selected, on_select);
        for (segment, (image, _)) in control.segments.iter_mut().zip(choices) {
            segment.symbol = Some((Symbol::new(image.clone()), Symbol::new(image)));
        }
        control
    }

    pub fn fill_equally(mut self) -> Self {
        self.fill = true;
        self
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn set_selected(&mut self, cx: &Context, index: usize) {
        self.glide_to(cx, index);
    }

    fn segment(&self, index: usize) -> Rect {
        let x = self.segments[..index]
            .iter()
            .map(|segment| segment.width)
            .sum::<f32>();
        Rect::new(
            self.frame.x + x,
            self.frame.y,
            self.segments[index].width,
            self.frame.height,
        )
    }

    fn select(&mut self, cx: &Context, index: usize) {
        if self.glide_to(cx, index) {
            (self.on_select)(index);
        }
    }

    fn glide_to(&mut self, cx: &Context, index: usize) -> bool {
        if index >= self.segments.len() || self.selected == Some(index) {
            return false;
        }
        self.selected = Some(index);
        if self.frame.width <= 0.0 {
            self.pill = None;
            return true;
        }
        let target = self.segment(index);
        let offset = target.x - self.frame.x;
        match &mut self.pill {
            Some((x, width)) => {
                cx.animate(x, offset, Spring::QUICK);
                cx.animate(width, target.width, Spring::QUICK);
            }
            None => self.pill = Some((Animated::points(offset), Animated::points(target.width))),
        }
        true
    }

    fn pill(&self, cx: &Context) -> Option<Rect> {
        let index = self.selected?;
        let target = self.segment(index);
        let (x, width) = match &self.pill {
            Some((x, width)) => (self.frame.x + cx.sample(x), cx.sample(width)),
            None => (target.x, target.width),
        };
        Some(Rect::new(x, self.frame.y, width, self.frame.height).inset(INSET))
    }
    fn paint_segments(&self, cx: &Context, items: &mut Vec<Item>) {
        let track = Shape::rounded(self.frame, Radius::M);
        items.push(Item::Fill {
            shape: track,
            paint: cx.palette.well.into(),
        });
        if cx.is_focused(self.id) {
            super::focus_ring(cx, track, items);
        }
        let centred = |paragraph: &Paragraph, segment: Rect| {
            Point::new(
                (segment.x + (segment.width - paragraph.width()) / 2.0).round(),
                segment.y + ((segment.height - paragraph.height()) / 2.0).round(),
            )
        };
        let symbol_rect = |segment: Rect| {
            Rect::new(
                (segment.x + (segment.width - SYMBOL_SIZE) / 2.0).round(),
                (segment.y + (segment.height - SYMBOL_SIZE) / 2.0).round(),
                SYMBOL_SIZE,
                SYMBOL_SIZE,
            )
        };
        for (index, segment) in self.segments.iter().enumerate() {
            if let Some((plain, _)) = &segment.symbol {
                plain.paint(symbol_rect(self.segment(index)), items);
            }
            if let Some(paragraph) = &segment.plain {
                items.push(Item::Text {
                    origin: centred(paragraph, self.segment(index)),
                    paragraph: paragraph.clone(),
                });
            }
        }
        let Some(pill) = self.pill(cx) else {
            return;
        };
        let shape = Shape::rounded(pill, Radius::concentric(Radius::M, INSET));
        items.push(Item::Shadow {
            shape,
            offset: Point::new(0.0, 1.0),
            blur_radius: 1.5,
            color: Color::BLACK.with_opacity(CHIP_SHADOW),
        });
        let mut selected = vec![Item::Fill {
            shape,
            paint: cx.palette.raised.into(),
        }];
        for (index, segment) in self.segments.iter().enumerate() {
            if let Some((_, symbol)) = &segment.symbol {
                symbol.paint(symbol_rect(self.segment(index)), &mut selected);
            }
            if let Some(paragraph) = &segment.selected {
                selected.push(Item::Text {
                    origin: centred(paragraph, self.segment(index)),
                    paragraph: paragraph.clone(),
                });
            }
        }
        items.push(Item::Clip {
            shape,
            items: selected,
        });
    }
}

impl Widget for SegmentedControl {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        for segment in &mut self.segments {
            if let Some((plain, selected)) = &mut segment.symbol {
                plain.tint(cx.palette.ink2);
                selected.tint(cx.palette.ink);
                segment.width = SYMBOL_SEGMENT;
                continue;
            }
            let style = TextStyle::new(FONT_SIZE, cx.palette.ink2);
            let plain = cx
                .text
                .paragraph(&segment.title, style, None, TextAlign::Start);
            segment.width = (plain.width() + 2.0 * PADDING).round();
            segment.selected = Some(cx.text.paragraph(
                &segment.title,
                TextStyle::new(FONT_SIZE, cx.palette.ink).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            ));
            segment.plain = Some(plain);
        }
        let width = match self.fill {
            true => {
                self.segments
                    .iter()
                    .map(|segment| segment.width)
                    .fold(0.0, f32::max)
                    * self.segments.len() as f32
            }
            false => self.segments.iter().map(|segment| segment.width).sum(),
        };
        Size::new(width, cx.control_height(ControlSize::Regular))
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        if self.fill && !self.segments.is_empty() {
            let width = frame.width / self.segments.len() as f32;
            for segment in &mut self.segments {
                segment.width = width;
            }
        }
        let width = self.segments.iter().map(|segment| segment.width).sum();
        let resized = width != self.frame.width;
        self.frame = Rect::new(
            frame.x,
            frame.y,
            width,
            cx.control_height(ControlSize::Regular),
        );
        if let Some(index) = self.selected
            && (self.pill.is_none() || resized)
        {
            let target = self.segment(index);
            self.pill = Some((
                Animated::points(target.x - self.frame.x),
                Animated::points(target.width),
            ));
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if !self.enabled {
            let mut dimmed = Vec::new();
            self.paint_segments(cx, &mut dimmed);
            items.push(Item::Opacity {
                alpha: DISABLED_ALPHA,
                items: dimmed,
            });
            return;
        }
        self.paint_segments(cx, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::RadioGroup);
        node.set_label(self.label.clone());
        if let Some(segment) = self.selected.and_then(|index| self.segments.get(index)) {
            node.set_value(segment.title.clone());
        }
        match self.enabled {
            true => {
                node.add_action(Action::Increment);
                node.add_action(Action::Decrement);
            }
            false => node.set_disabled(),
        }
        node
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        if !self.enabled {
            return false;
        }
        let current = self.selected.unwrap_or(0);
        let index = match request.action {
            Action::Increment => current + 1,
            Action::Decrement => current.saturating_sub(1),
            _ => return false,
        };
        self.select(cx, index);
        true
    }

    fn parts(&self, _cx: &Context) -> Parts {
        let nodes: Vec<_> = self
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                let rect = self.segment(index);
                let mut node = Node::new(Role::RadioButton);
                node.set_label(segment.title.clone());
                node.set_toggled(match self.selected == Some(index) {
                    true => Toggled::True,
                    false => Toggled::False,
                });
                node.add_action(Action::Click);
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
            focus: self.selected.map(|index| self.id.part(index as u64)),
            nodes,
        }
    }

    fn perform_part(&mut self, cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        if !self.enabled || request.action != Action::Click {
            return false;
        }
        self.select(cx, part as usize);
        true
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        if !self.enabled {
            return false;
        }
        match event {
            Event::PointerDown(pointer) if pointer.is_primary() => {
                if let Some(index) = (0..self.segments.len())
                    .find(|index| self.segment(*index).contains(pointer.position))
                {
                    self.select(cx, index);
                }
            }
            Event::Key { key: Key::Left, .. } => {
                let index = self.selected.map_or(0, |index| index.saturating_sub(1));
                self.select(cx, index);
            }
            Event::Key {
                key: Key::Right, ..
            } => {
                let index = self.selected.map_or(0, |index| index + 1);
                self.select(cx, index);
            }
            _ => return false,
        }
        true
    }

    fn accepts_focus(&self, cx: &Context) -> bool {
        self.enabled && cx.keyboard_navigation
    }
}
