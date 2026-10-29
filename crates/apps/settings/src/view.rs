use std::collections::HashMap;

use unbound_toolkit::ui::{Event, Key as Pressed, Modifiers};
use unbound_toolkit::{
    Color, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

use crate::model::{Control, Key, Pane, Value, panes, search};

pub const SIDEBAR_WIDTH: f32 = 220.0;
const MARGIN: f32 = 12.0;
const SEARCH_HEIGHT: f32 = 28.0;
const PANE_ROW: f32 = 28.0;
const PANES_TOP: f32 = 52.0;
const CONTENT_INSET: f32 = 24.0;
const TITLE_SIZE: f32 = 20.0;
const LABEL_SIZE: f32 = 13.0;
const CONTROLS_TOP: f32 = 64.0;
const CONTROL_ROW: f32 = 68.0;
const SEGMENT_TOP: f32 = 24.0;
const SEGMENT_HEIGHT: f32 = 28.0;
const SEGMENT_PADDING: f32 = 12.0;
const FOCUS_RING: f32 = 3.0;

pub struct SettingsView {
    panes: Vec<Pane>,
    values: HashMap<Key, Value>,
    query: String,
    pane: usize,
    focus: Option<usize>,
    changes: Vec<(Key, Value)>,
    message: Option<String>,
    palette: Palette,
    text: TextSystem,
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.y >= rect.y
        && point.x < rect.x + rect.width
        && point.y < rect.y + rect.height
}

impl SettingsView {
    pub fn new(palette: Palette, text: TextSystem) -> Self {
        Self {
            panes: panes(),
            values: HashMap::new(),
            query: String::new(),
            pane: 0,
            focus: None,
            changes: Vec::new(),
            message: None,
            palette,
            text,
        }
    }

    pub fn panes(&self) -> &[Pane] {
        &self.panes
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn set_value(&mut self, key: Key, value: Value) {
        self.values.insert(key, value);
    }

    pub fn value(&self, key: Key) -> Option<&Value> {
        self.values.get(&key)
    }

    pub fn set_message(&mut self, message: Option<String>) {
        self.message = message;
    }

    pub fn take_changes(&mut self) -> Vec<(Key, Value)> {
        std::mem::take(&mut self.changes)
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected_pane(&self) -> &Pane {
        &self.panes[self.pane]
    }

    pub fn focused_control(&self) -> Option<&Control> {
        self.selected_pane().controls.get(self.focus?)
    }

    pub fn visible_panes(&self) -> Vec<usize> {
        if self.query.trim().is_empty() {
            (0..self.panes.len()).collect()
        } else {
            search(&self.panes, &self.query)
                .into_iter()
                .map(|found| found.pane)
                .collect()
        }
    }

    pub fn highlighted_controls(&self) -> Vec<usize> {
        if self.query.trim().is_empty() {
            return Vec::new();
        }
        search(&self.panes, &self.query)
            .into_iter()
            .find(|found| found.pane == self.pane)
            .map(|found| found.controls)
            .unwrap_or_default()
    }

    fn select_pane(&mut self, pane: usize) {
        if pane != self.pane {
            self.pane = pane;
            self.focus = None;
        }
    }

    fn follow_query(&mut self) {
        let visible = self.visible_panes();
        if let Some(&first) = visible.first()
            && !visible.contains(&self.pane)
        {
            self.select_pane(first);
        }
    }

    fn step_pane(&mut self, forward: bool) -> bool {
        let visible = self.visible_panes();
        let Some(at) = visible.iter().position(|&pane| pane == self.pane) else {
            return false;
        };
        let next = if forward {
            visible.get(at + 1)
        } else {
            at.checked_sub(1).and_then(|before| visible.get(before))
        };
        next.copied().is_some_and(|pane| {
            self.select_pane(pane);
            true
        })
    }

    fn step_focus(&mut self, forward: bool) {
        let count = self.selected_pane().controls.len();
        self.focus = match (self.focus, forward) {
            (None, true) => Some(0),
            (None, false) => count.checked_sub(1),
            (Some(at), true) => (at + 1 < count).then_some(at + 1),
            (Some(at), false) => at.checked_sub(1),
        };
    }

    fn choose(&mut self, control: usize, choice: usize) {
        let Some(control) = self.selected_pane().controls.get(control) else {
            return;
        };
        let Some(value) = control
            .choices
            .get(choice)
            .map(|choice| choice.value.clone())
        else {
            return;
        };
        let key = control.key;
        if self.values.get(&key) != Some(&value) {
            self.values.insert(key, value.clone());
            self.changes.push((key, value));
        }
    }

    fn step_choice(&mut self, forward: bool) -> bool {
        let Some(at) = self.focus else {
            return false;
        };
        let control = &self.selected_pane().controls[at];
        let count = control.choices.len();
        let current = self
            .values
            .get(&control.key)
            .and_then(|value| control.position(value));
        let next = match (current, forward) {
            (None, _) => 0,
            (Some(index), true) => (index + 1).min(count - 1),
            (Some(index), false) => index.saturating_sub(1),
        };
        self.choose(at, next);
        true
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Key { key, modifiers } => self.key(*key, *modifiers),
            Event::PointerDown(at) => self.click(*at),
            _ => false,
        }
    }

    fn key(&mut self, key: Pressed, modifiers: Modifiers) -> bool {
        if modifiers.logo || modifiers.control || modifiers.alt {
            return false;
        }
        match key {
            Pressed::Character(character) => {
                self.query.push(character);
                self.follow_query();
            }
            Pressed::Space if self.focus.is_none() => {
                self.query.push(' ');
            }
            Pressed::Backspace => {
                if self.query.pop().is_none() {
                    return false;
                }
                self.follow_query();
            }
            Pressed::Escape if !self.query.is_empty() => {
                self.query.clear();
            }
            Pressed::Up => return self.step_pane(false),
            Pressed::Down => return self.step_pane(true),
            Pressed::Tab => self.step_focus(!modifiers.shift),
            Pressed::Left => return self.step_choice(false),
            Pressed::Right => return self.step_choice(true),
            _ => return false,
        }
        true
    }

    fn pane_row(index: usize) -> Rect {
        Rect::new(
            MARGIN,
            PANES_TOP + index as f32 * PANE_ROW,
            SIDEBAR_WIDTH - 2.0 * MARGIN,
            PANE_ROW,
        )
    }

    fn segment_rects(&mut self, control: usize) -> Vec<Rect> {
        let y = CONTROLS_TOP + control as f32 * CONTROL_ROW + SEGMENT_TOP;
        let mut x = SIDEBAR_WIDTH + CONTENT_INSET;
        let labels = self.panes[self.pane].controls[control]
            .choices
            .iter()
            .map(|choice| choice.label)
            .collect::<Vec<_>>();
        labels
            .into_iter()
            .map(|label| {
                let width = self.paragraph(label, LABEL_SIZE, Color::hex(0)).width()
                    + 2.0 * SEGMENT_PADDING;
                let rect = Rect::new(x, y, width.round(), SEGMENT_HEIGHT);
                x += rect.width;
                rect
            })
            .collect()
    }

    fn click(&mut self, at: Point) -> bool {
        if at.x < SIDEBAR_WIDTH {
            let visible = self.visible_panes();
            return visible
                .iter()
                .enumerate()
                .find(|(row, _)| contains(Self::pane_row(*row), at))
                .map(|(_, &pane)| pane)
                .is_some_and(|pane| {
                    self.select_pane(pane);
                    true
                });
        }
        for control in 0..self.selected_pane().controls.len() {
            if let Some(choice) = self
                .segment_rects(control)
                .iter()
                .position(|segment| contains(*segment, at))
            {
                self.focus = Some(control);
                self.choose(control, choice);
                return true;
            }
        }
        false
    }

    fn paragraph(&mut self, text: &str, size: f32, color: Color) -> Paragraph {
        self.text
            .paragraph(text, TextStyle::new(size, color), None, TextAlign::Start)
    }

    fn centred_text(&mut self, text: &str, size: f32, color: Color, rect: Rect) -> Item {
        let paragraph = self.paragraph(text, size, color);
        Item::Text {
            origin: Point::new(
                rect.x + ((rect.width - paragraph.width()) / 2.0).round(),
                rect.y + ((rect.height - paragraph.height()) / 2.0).round(),
            ),
            paragraph,
        }
    }

    fn paint_sidebar(&mut self, height: f32, items: &mut Vec<Item>) {
        let palette = self.palette;
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(0.0, 0.0, SIDEBAR_WIDTH, height)),
            paint: palette.surface_elevated.into(),
        });
        let field = Rect::new(MARGIN, MARGIN, SIDEBAR_WIDTH - 2.0 * MARGIN, SEARCH_HEIGHT);
        items.push(Item::Fill {
            shape: Shape::rounded(field, 7.0),
            paint: palette.label_muted.with_alpha(36).into(),
        });
        let (shown, color) = if self.query.is_empty() {
            ("Search".to_owned(), palette.label_muted)
        } else {
            (self.query.clone(), palette.label_primary)
        };
        let query = self.paragraph(&shown, LABEL_SIZE, color);
        items.push(Item::Text {
            origin: Point::new(
                field.x + 10.0,
                field.y + ((field.height - query.height()) / 2.0).round(),
            ),
            paragraph: query,
        });
        for (row, pane) in self.visible_panes().into_iter().enumerate() {
            let rect = Self::pane_row(row);
            let selected = pane == self.pane;
            if selected {
                items.push(Item::Fill {
                    shape: Shape::rounded(rect, 6.0),
                    paint: palette.accent_color.into(),
                });
            }
            let color = if selected {
                Color::hex(0xFFFFFF)
            } else {
                palette.label_primary
            };
            let name = self.paragraph(self.panes[pane].name, LABEL_SIZE, color);
            items.push(Item::Text {
                origin: Point::new(
                    rect.x + 10.0,
                    rect.y + ((rect.height - name.height()) / 2.0).round(),
                ),
                paragraph: name,
            });
        }
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(SIDEBAR_WIDTH - 1.0, 0.0, 1.0, height)),
            paint: palette.separator_color.into(),
        });
    }

    fn paint_control(&mut self, index: usize, highlighted: bool, items: &mut Vec<Item>) {
        let palette = self.palette;
        let control = self.panes[self.pane].controls[index].clone();
        let x = SIDEBAR_WIDTH + CONTENT_INSET;
        let y = CONTROLS_TOP + index as f32 * CONTROL_ROW;
        let label = self.paragraph(control.label, LABEL_SIZE, palette.label_primary);
        items.push(Item::Text {
            origin: Point::new(x, y),
            paragraph: label,
        });
        let segments = self.segment_rects(index);
        let (Some(first), Some(last)) = (segments.first(), segments.last()) else {
            return;
        };
        let track = Rect::new(
            first.x,
            first.y,
            last.x + last.width - first.x,
            SEGMENT_HEIGHT,
        );
        items.push(Item::Fill {
            shape: Shape::rounded(track, 7.0),
            paint: palette.label_muted.with_alpha(36).into(),
        });
        if highlighted {
            items.push(Item::Stroke {
                shape: Shape::rounded(track.inset(-4.0), 10.0),
                width: 2.0,
                paint: palette.accent_color.into(),
            });
        }
        if self.focus == Some(index) {
            items.push(Item::Stroke {
                shape: Shape::rounded(track.inset(-FOCUS_RING / 2.0), 7.0 + FOCUS_RING / 2.0),
                width: FOCUS_RING,
                paint: palette.accent_color.with_alpha(128).into(),
            });
        }
        let chosen = self
            .values
            .get(&control.key)
            .and_then(|value| control.position(value));
        for (position, (segment, choice)) in segments.iter().zip(&control.choices).enumerate() {
            let color = if chosen == Some(position) {
                items.push(Item::Fill {
                    shape: Shape::rounded(segment.inset(2.0), 5.0),
                    paint: palette.accent_color.into(),
                });
                Color::hex(0xFFFFFF)
            } else {
                palette.label_primary
            };
            let text = self.centred_text(choice.label, LABEL_SIZE, color, *segment);
            items.push(text);
        }
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let palette = self.palette;
        let mut items = Vec::new();
        self.paint_sidebar(height as f32, &mut items);
        let pane_name = self.panes[self.pane].name;
        let title = self.text.paragraph(
            pane_name,
            TextStyle::new(TITLE_SIZE, palette.label_primary).weight(TextStyle::SEMIBOLD),
            None,
            TextAlign::Start,
        );
        items.push(Item::Text {
            origin: Point::new(SIDEBAR_WIDTH + CONTENT_INSET, 20.0),
            paragraph: title,
        });
        let highlighted = self.highlighted_controls();
        for index in 0..self.panes[self.pane].controls.len() {
            self.paint_control(index, highlighted.contains(&index), &mut items);
        }
        if let Some(message) = self.message.clone() {
            let text = self.paragraph(&message, LABEL_SIZE, palette.label_muted);
            items.push(Item::Text {
                origin: Point::new(
                    SIDEBAR_WIDTH + CONTENT_INSET,
                    height as f32 - CONTENT_INSET - text.height(),
                ),
                paragraph: text,
            });
        }
        Scene {
            width,
            height,
            background: palette.surface_background,
            items,
        }
    }
}
