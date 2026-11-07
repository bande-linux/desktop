use std::time::Instant;

use super::reorder::{Reorder, ReorderSlot};
use super::{Pointer, PointerButton};
use bande_graphics::{
    Color, Cut, Item, Palette, Path, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
    TextSystem, shortened,
};

pub const TAB_HEIGHT: f32 = 28.0;
const TRAILING: f32 = 12.0;
const NEW_TAB_SIZE: f32 = 28.0;
const NEW_TAB_GAP: f32 = 8.0;
const TAB_GAP: f32 = 4.0;
const CLOSE_SIZE: f32 = 16.0;
const CLOSE_INSET: f32 = 6.0;
const LABEL_INSET: f32 = 26.0;
const DRAG_SLOP: f32 = 4.0;
const HOVER_OPACITY: f32 = 0.5;
const PRESSED_OPACITY: f32 = 0.12;
const BADGE: f32 = 6.0;
const GLYPH_STROKE: f32 = 1.5;
const CROSS_ARM: f32 = 3.5;
const PLUS_ARM: f32 = 5.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabTarget {
    Select(usize),
    Close(usize),
    NewTab,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabStripLayout {
    pub tabs: Vec<Rect>,
    pub new_tab: Rect,
    pub height: f32,
}

impl TabStripLayout {
    pub fn new(count: usize, width: f32, leading: f32, height: f32) -> Self {
        let top = ((height - TAB_HEIGHT) / 2.0).round();
        let new_tab_x = (width - TRAILING - NEW_TAB_SIZE).max(leading);
        let new_tab = Rect::new(new_tab_x, top, NEW_TAB_SIZE, NEW_TAB_SIZE);
        let shown = if count > 1 { count } else { 0 };
        let available = (new_tab_x - NEW_TAB_GAP - leading).max(0.0);
        let pitch = (available + TAB_GAP) / shown.max(1) as f32;
        let tabs = (0..shown)
            .map(|index| {
                let left = (leading + index as f32 * pitch).round();
                let right = (leading + (index + 1) as f32 * pitch - TAB_GAP).round();
                Rect::new(left, top, (right - left).max(0.0), TAB_HEIGHT)
            })
            .collect();
        Self {
            tabs,
            new_tab,
            height,
        }
    }

    pub fn close_button(tab: Rect) -> Rect {
        Rect::new(
            tab.x + CLOSE_INSET,
            tab.y + ((tab.height - CLOSE_SIZE) / 2.0).round(),
            CLOSE_SIZE,
            CLOSE_SIZE,
        )
    }

    pub fn hit(&self, point: Point) -> Option<TabTarget> {
        if self.new_tab.contains(point) {
            return Some(TabTarget::NewTab);
        }
        let index = self.tabs.iter().position(|tab| tab.contains(point))?;
        if Self::close_button(self.tabs[index]).contains(point) {
            Some(TabTarget::Close(index))
        } else {
            Some(TabTarget::Select(index))
        }
    }

    pub fn insertion_index(&self, x: f32) -> usize {
        self.tabs
            .iter()
            .position(|slot| x < slot.x + slot.width / 2.0)
            .unwrap_or(self.tabs.len())
    }

    pub fn grab_point(&self, grab: Point) -> Point {
        let x = self.tabs.first().map_or(0.0, |slot| slot.x);
        Point::new(x + grab.x, grab.y)
    }

    fn slots(&self) -> Vec<ReorderSlot> {
        self.tabs
            .iter()
            .map(|tab| ReorderSlot {
                x: tab.x,
                width: tab.width,
            })
            .collect()
    }

    fn contains_y(&self, y: f32) -> bool {
        (0.0..self.height).contains(&y)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TabAction {
    Select(usize),
    Close(usize),
    NewTab,
    Menu { tab: Option<usize>, at: Point },
    Moved { from: usize, to: usize },
    Detach { index: usize, grab: Point },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Gesture {
    Pressed(TabTarget, Point),
    Reordering,
}

pub struct TabLabel<'a> {
    pub title: &'a str,
    pub badge: bool,
}

pub struct TabsShown<'a> {
    pub labels: &'a [TabLabel<'a>],
    pub active: usize,
    pub focused: bool,
    pub palette: &'a Palette,
}

#[derive(Default)]
pub struct TabStrip {
    ids: Vec<u64>,
    reorder: Reorder,
    hover: Option<TabTarget>,
    gesture: Option<Gesture>,
}

impl TabStrip {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sync(&mut self, ids: &[u64]) {
        if self.ids == ids {
            return;
        }
        self.reorder.rearrange(
            ids.iter()
                .map(|id| self.ids.iter().position(|known| known == id)),
        );
        self.ids = ids.to_vec();
        if self.ids.len() < 2 {
            self.gesture = None;
            self.hover = None;
            self.reorder.take_lifted();
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.reorder.lifted().is_some()
    }

    pub fn is_engaged(&self) -> bool {
        self.gesture.is_some()
    }

    pub fn hover(&mut self, layout: &TabStripLayout, at: Option<Point>) -> bool {
        let hover = at.and_then(|at| layout.hit(at));
        let changed = hover != self.hover;
        self.hover = hover;
        changed
    }

    pub fn press(&mut self, layout: &TabStripLayout, pointer: &Pointer) -> Option<TabAction> {
        let at = pointer.position;
        let target = layout.hit(at)?;
        if pointer.button == PointerButton::Secondary {
            let tab = match target {
                TabTarget::Select(index) | TabTarget::Close(index) => Some(index),
                TabTarget::NewTab => None,
            };
            return Some(TabAction::Menu { tab, at });
        }
        if pointer.button != PointerButton::Primary {
            return None;
        }
        self.gesture = Some(Gesture::Pressed(target, at));
        match target {
            TabTarget::Select(index) => Some(TabAction::Select(index)),
            TabTarget::Close(_) | TabTarget::NewTab => None,
        }
    }

    pub fn moved(
        &mut self,
        layout: &TabStripLayout,
        at: Point,
        now: Instant,
        reduce_motion: bool,
    ) -> Option<TabAction> {
        match self.gesture? {
            Gesture::Pressed(TabTarget::Select(index), from)
                if (at.x - from.x).hypot(at.y - from.y) > DRAG_SLOP =>
            {
                let grab = from.x - layout.tabs.get(index).map_or(from.x, |tab| tab.x);
                self.reorder.lift(&layout.slots(), index, grab);
                self.gesture = Some(Gesture::Reordering);
                self.drag(layout, at.x, now, reduce_motion)
            }
            Gesture::Reordering if layout.tabs.len() > 1 && !layout.contains_y(at.y) => {
                self.gesture = None;
                let lifted = self.reorder.take_lifted()?;
                Some(TabAction::Detach {
                    index: lifted.index,
                    grab: Point::new(lifted.grab, layout.height / 2.0),
                })
            }
            Gesture::Reordering => self.drag(layout, at.x, now, reduce_motion),
            Gesture::Pressed(..) => None,
        }
    }

    pub fn released(
        &mut self,
        layout: &TabStripLayout,
        at: Point,
        now: Instant,
        reduce_motion: bool,
    ) -> Option<TabAction> {
        match self.gesture.take()? {
            Gesture::Pressed(target, _) if layout.hit(at) == Some(target) => match target {
                TabTarget::Close(index) => Some(TabAction::Close(index)),
                TabTarget::NewTab => Some(TabAction::NewTab),
                TabTarget::Select(_) => None,
            },
            Gesture::Reordering => {
                self.reorder.settle(&layout.slots(), now, reduce_motion);
                None
            }
            Gesture::Pressed(..) => None,
        }
    }

    pub fn cancel(&mut self, layout: &TabStripLayout, now: Instant, reduce_motion: bool) {
        if matches!(self.gesture.take(), Some(Gesture::Reordering)) {
            self.reorder.settle(&layout.slots(), now, reduce_motion);
        }
    }

    pub fn slide_to(
        &mut self,
        layout: &TabStripLayout,
        index: usize,
        x: f32,
        now: Instant,
        reduce_motion: bool,
    ) -> Option<TabAction> {
        let grab = layout.tabs.get(index).map_or(0.0, |slot| slot.width / 2.0);
        self.reorder.lift(&layout.slots(), index, grab);
        let moved = self.drag(layout, x, now, reduce_motion);
        self.reorder.settle(&layout.slots(), now, reduce_motion);
        moved
    }

    fn drag(
        &mut self,
        layout: &TabStripLayout,
        x: f32,
        now: Instant,
        reduce_motion: bool,
    ) -> Option<TabAction> {
        let run = 0..=layout.tabs.len().checked_sub(1)?;
        let (from, to) = self
            .reorder
            .drag_to(&layout.slots(), run, x, now, reduce_motion)?;
        if from < self.ids.len() && to < self.ids.len() {
            let id = self.ids.remove(from);
            self.ids.insert(to, id);
        }
        Some(TabAction::Moved { from, to })
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.reorder.animating(now)
    }

    pub fn paint(
        &self,
        layout: &TabStripLayout,
        tabs: &TabsShown<'_>,
        text: &mut TextSystem,
        now: Instant,
    ) -> Vec<Item> {
        let TabsShown {
            labels,
            active,
            focused,
            palette,
        } = tabs;
        let (active, focused) = (*active, *focused);
        let lifted = self.reorder.lifted().and_then(|lifted| {
            layout
                .tabs
                .get(lifted.index)
                .map(|slot| (lifted.index, lifted.left - slot.x))
        });
        let hover = self.hover.filter(|_| lifted.is_none());
        let mut items = Vec::new();
        let order = (0..layout.tabs.len())
            .filter(|index| Some(*index) != lifted.map(|(index, _)| index))
            .chain(lifted.map(|(index, _)| index));
        for index in order {
            let (Some(slot), Some(label)) = (layout.tabs.get(index), labels.get(index)) else {
                continue;
            };
            let offset = match lifted {
                Some((lifted, dragged)) if lifted == index => dragged,
                _ => self
                    .reorder
                    .slide(index)
                    .map_or(0.0, |slide| slide.value_at(now).0),
            };
            let tab = Rect::new(slot.x + offset, slot.y, slot.width, slot.height);
            let selected = index == active;
            let hovered = matches!(
                hover,
                Some(TabTarget::Select(hovered) | TabTarget::Close(hovered)) if hovered == index
            );
            let fill = match (selected, hovered) {
                (true, _) => Some(palette.well),
                (false, true) => Some(palette.well.with_opacity(HOVER_OPACITY)),
                (false, false) => None,
            };
            if let Some(fill) = fill {
                items.push(Item::Fill {
                    shape: Shape::rounded(tab, Radius::S),
                    paint: fill.into(),
                });
            }
            let color = if selected && focused {
                palette.ink
            } else {
                palette.ink2
            };
            let weight = if selected {
                TextStyle::MEDIUM
            } else {
                TextStyle::REGULAR
            };
            let area = Rect::new(
                tab.x + LABEL_INSET,
                tab.y,
                (tab.width - 2.0 * LABEL_INSET).max(0.0),
                tab.height,
            );
            items.push(centred(text, label.title, weight, color, area));
            let close = TabStripLayout::close_button(tab);
            if label.badge && !selected && !hovered {
                let mark = Rect::new(
                    close.x + ((close.width - BADGE) / 2.0).round(),
                    close.y + ((close.height - BADGE) / 2.0).round(),
                    BADGE,
                    BADGE,
                );
                items.push(Item::Fill {
                    shape: Shape::rounded(mark, 1.0),
                    paint: palette.accent.into(),
                });
            }
            if hovered {
                if hover == Some(TabTarget::Close(index)) {
                    items.push(Item::Fill {
                        shape: Shape::rounded(close, Radius::XS),
                        paint: palette.ink.with_opacity(PRESSED_OPACITY).into(),
                    });
                }
                items.extend(cross(close, palette.ink2));
            }
        }
        let new_tab = layout.new_tab;
        let hovered = hover == Some(TabTarget::NewTab);
        if hovered {
            items.push(Item::Fill {
                shape: Shape::rounded(new_tab, Radius::S),
                paint: palette.well.into(),
            });
        }
        items.extend(plus(
            new_tab,
            if hovered { palette.ink } else { palette.ink2 },
        ));
        items
    }
}

fn centred(text: &mut TextSystem, title: &str, weight: f32, color: Color, area: Rect) -> Item {
    let style = TextStyle::role(TextRole::Body, color).weight(weight);
    let shown = shortened(title, Cut::End, |candidate| {
        text.paragraph(candidate, style, None, TextAlign::Start)
            .width()
            <= area.width
    });
    let paragraph = text.paragraph(&shown, style, None, TextAlign::Start);
    Item::Text {
        origin: Point::new(
            (area.x + (area.width - paragraph.width()) / 2.0).round(),
            (area.y + (area.height - paragraph.height()) / 2.0).round(),
        ),
        paragraph,
    }
}

fn stroke(path: Path, color: Color) -> Item {
    Item::StrokePath {
        path,
        width: GLYPH_STROKE,
        paint: color.into(),
    }
}

fn glyph(rect: Rect, color: Color, arms: [(f32, f32); 2]) -> [Item; 2] {
    let middle = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let at = |x: f32, y: f32| Point::new(middle.x + x, middle.y + y);
    arms.map(|(x, y)| stroke(Path::through(&[at(-x, -y), at(x, y)]), color))
}

fn cross(rect: Rect, color: Color) -> [Item; 2] {
    glyph(
        rect,
        color,
        [(CROSS_ARM, CROSS_ARM), (CROSS_ARM, -CROSS_ARM)],
    )
}

fn plus(rect: Rect, color: Color) -> [Item; 2] {
    glyph(rect, color, [(PLUS_ARM, 0.0), (0.0, PLUS_ARM)])
}
