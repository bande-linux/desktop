use std::cell::Cell;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use accesskit::{Node, Role};
use bande_graphics::{Item, Palette, Point, Rect, Shape};

use super::{Animated, Context, Size, Spring, Widget, WidgetId};

pub const PROGRESS_HEIGHT: f32 = 6.0;
pub const SPINNER_SIZE: f32 = 16.0;
const SWEEP: Duration = Duration::from_millis(1600);
const SWEEP_CELLS: f32 = 4.0;
const SPOKES: u32 = 12;
const SPIN: Duration = Duration::from_millis(1000);
pub const PROGRESS_CELLS: usize = 24;
pub const PROGRESS_CELL_GAP: f32 = 2.0;
const CELL_RADIUS: f32 = 1.0;

pub fn progress_cells_for(
    palette: &Palette,
    frame: Rect,
    lit: impl Fn(f32) -> f32,
    items: &mut Vec<Item>,
) {
    let width =
        (frame.width - PROGRESS_CELL_GAP * (PROGRESS_CELLS - 1) as f32) / PROGRESS_CELLS as f32;
    for index in 0..PROGRESS_CELLS {
        let shape = Shape::rounded(
            Rect::new(
                frame.x + index as f32 * (width + PROGRESS_CELL_GAP),
                frame.y,
                width,
                frame.height,
            ),
            CELL_RADIUS,
        );
        items.push(Item::Fill {
            shape,
            paint: palette.well.into(),
        });
        let fill = lit(index as f32).clamp(0.0, 1.0);
        if fill > 0.0 {
            items.push(Item::Fill {
                shape,
                paint: palette.accent.with_opacity(fill).into(),
            });
        }
    }
}

static MOVING_SINCE: OnceLock<Instant> = OnceLock::new();

fn phase(cx: &Context, period: Duration) -> f32 {
    let since = *MOVING_SINCE.get_or_init(|| cx.now());
    let elapsed = cx.now().saturating_duration_since(since);
    (elapsed.as_secs_f64() / period.as_secs_f64()).fract() as f32
}

fn eased(phase: f32) -> f32 {
    (1.0 - (phase * TAU).cos()) / 2.0
}

pub fn indeterminate_sweep(phase: f32) -> impl Fn(f32) -> f32 {
    let start = eased(phase) * (PROGRESS_CELLS as f32 - SWEEP_CELLS);
    move |cell: f32| {
        (cell + 1.0 - start)
            .min(start + SWEEP_CELLS - cell)
            .min(1.0)
    }
}

pub struct ProgressBar {
    id: WidgetId,
    label: String,
    value: Option<f32>,
    shown: Animated,
    frame: Rect,
}

impl ProgressBar {
    pub fn new(label: &str, value: Option<f32>) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            value: value.map(|value| value.clamp(0.0, 1.0)),
            shown: Animated::new(value.unwrap_or_default().clamp(0.0, 1.0)),
            frame: Rect::default(),
        }
    }

    pub fn value(&self) -> Option<f32> {
        self.value
    }

    pub fn set_value(&mut self, cx: &Context, value: Option<f32>) {
        let value = value.map(|value| value.clamp(0.0, 1.0));
        match (self.value, value) {
            (None, Some(value)) => self.shown.jump_to(value),
            (_, Some(value)) => cx.animate(&mut self.shown, value, Spring::QUICK),
            _ => {}
        }
        self.value = value;
    }

    fn sweep(cx: &Context) -> impl Fn(f32) -> f32 {
        let phase = phase(cx, SWEEP);
        cx.animating.set(true);
        let reduce_motion = cx.reduce_motion;
        let eased = eased(phase);
        let swept = indeterminate_sweep(phase);
        move |cell: f32| match reduce_motion {
            true => 0.35 + 0.35 * eased,
            false => swept(cell),
        }
    }
}

impl Widget for ProgressBar {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, PROGRESS_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, PROGRESS_HEIGHT);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        match self.value {
            None => progress_cells_for(&cx.palette, self.frame, Self::sweep(cx), items),
            Some(_) => {
                let lit = cx.sample(&self.shown) * PROGRESS_CELLS as f32;
                progress_cells_for(&cx.palette, self.frame, |cell| lit - cell, items);
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ProgressIndicator);
        node.set_label(self.label.clone());
        match self.value {
            Some(value) => {
                node.set_numeric_value(f64::from(value) * 100.0);
                node.set_min_numeric_value(0.0);
                node.set_max_numeric_value(100.0);
            }
            None => node.set_busy(),
        }
        node
    }
}

pub struct Spinner {
    id: WidgetId,
    label: String,
    frame: Rect,
}

impl Spinner {
    pub fn new(label: &str) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            frame: Rect::default(),
        }
    }
}

impl Widget for Spinner {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, _available: Size) -> Size {
        Size::new(SPINNER_SIZE, SPINNER_SIZE)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, SPINNER_SIZE, SPINNER_SIZE);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        cx.animating.set(true);
        let lead = (phase(cx, SPIN) * SPOKES as f32) as u32;
        let centre = Point::new(
            self.frame.x + self.frame.width / 2.0,
            self.frame.y + self.frame.height / 2.0,
        );
        let (inner, outer) = (self.frame.width * 0.24, self.frame.width * 0.46);
        for spoke in 0..SPOKES {
            let angle = spoke as f32 / SPOKES as f32 * TAU - TAU / 4.0;
            let (sin, cos) = angle.sin_cos();
            let behind = (lead + SPOKES - spoke) % SPOKES;
            let alpha = 1.0 - 0.75 * behind as f32 / (SPOKES - 1) as f32;
            items.push(Item::Line {
                from: Point::new(centre.x + cos * inner, centre.y + sin * inner),
                to: Point::new(centre.x + cos * outer, centre.y + sin * outer),
                width: self.frame.width * 0.1,
                paint: cx.palette.ink2.with_alpha((alpha * 255.0) as u8).into(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::ProgressIndicator);
        node.set_label(self.label.clone());
        node.set_busy();
        node
    }
}

pub struct LevelIndicator {
    id: WidgetId,
    label: String,
    level: Rc<Cell<f32>>,
    frame: Rect,
}

impl LevelIndicator {
    pub fn new(label: &str, level: Rc<Cell<f32>>) -> Self {
        Self {
            id: WidgetId::next(),
            label: label.to_owned(),
            level,
            frame: Rect::default(),
        }
    }

    fn lit(&self) -> f32 {
        self.level.get().clamp(0.0, 1.0) * PROGRESS_CELLS as f32
    }
}

impl Widget for LevelIndicator {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        Size::new(available.width, PROGRESS_HEIGHT)
    }

    fn place(&mut self, _cx: &mut Context, frame: Rect) {
        self.frame = Rect::new(frame.x, frame.y, frame.width, PROGRESS_HEIGHT);
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let lit = self.lit();
        progress_cells_for(&cx.palette, self.frame, |cell| lit - cell, items);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Meter);
        node.set_label(self.label.clone());
        node.set_numeric_value(f64::from(self.level.get().clamp(0.0, 1.0)) * 100.0);
        node.set_min_numeric_value(0.0);
        node.set_max_numeric_value(100.0);
        node
    }
}

#[cfg(test)]
mod tests {
    use bande_graphics::{Scheme, TextSystem};

    use super::*;

    fn painted(cx: &mut Context, at: Instant, widget: &mut dyn Widget) -> Vec<Item> {
        cx.now = at;
        widget.measure(cx, Size::new(240.0, 40.0));
        widget.place(cx, Rect::new(0.0, 0.0, 240.0, 40.0));
        let mut items = Vec::new();
        widget.paint(cx, &mut items);
        items
    }

    #[test]
    fn a_bar_or_spinner_built_again_moves_on_from_where_it_was() {
        let mut cx = Context::new(TextSystem::new(), Palette::for_scheme(Scheme::Light));
        let start = Instant::now();
        let later = start + SWEEP * 3 / 8;
        let mut bar = ProgressBar::new("Installing", None);
        let mut spinner = Spinner::new("Reading");
        painted(&mut cx, start, &mut bar);
        painted(&mut cx, start, &mut spinner);
        assert_eq!(
            painted(&mut cx, later, &mut ProgressBar::new("Installing", None)),
            painted(&mut cx, later, &mut bar)
        );
        assert_eq!(
            painted(&mut cx, later, &mut Spinner::new("Reading")),
            painted(&mut cx, later, &mut spinner)
        );
    }
}
