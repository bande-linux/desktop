use std::time::{Duration, Instant};

use accesskit::{Action, Node, NodeId, Role, Toggled};
use unbound_graphics::{
    Color, Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use super::{Key, Modifiers, Size};

const ROW_HEIGHT: f32 = 22.0;
const SEPARATOR_HEIGHT: f32 = 11.0;
const VERTICAL_PADDING: f32 = 5.0;
const HIGHLIGHT_INSET: f32 = 5.0;
const TEXT_START: f32 = 24.0;
const TRAILING_PADDING: f32 = 16.0;
const SHORTCUT_GAP: f32 = 28.0;
const CHEVRON_SPACE: f32 = 18.0;
const MINIMUM_WIDTH: f32 = 120.0;
const RADIUS: f32 = 8.0;
const FONT_SIZE: f32 = 13.0;
pub const SUBMENU_DELAY: Duration = Duration::from_millis(200);
const TYPE_SELECT_PAUSE: Duration = Duration::from_secs(1);
const FLASH_STEP: Duration = Duration::from_millis(70);
const FADE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    pub items: Vec<MenuItem>,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self { items }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub title: String,
    pub action: String,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: bool,
    pub submenu: Option<Menu>,
    pub separator: bool,
}

impl MenuItem {
    pub fn new(title: &str, action: &str) -> Self {
        Self {
            title: title.to_owned(),
            action: action.to_owned(),
            shortcut: None,
            enabled: true,
            checked: false,
            submenu: None,
            separator: false,
        }
    }

    pub fn separator() -> Self {
        Self {
            separator: true,
            enabled: false,
            ..Self::new("", "")
        }
    }

    pub fn submenu(title: &str, menu: Menu) -> Self {
        Self {
            submenu: Some(menu),
            ..Self::new(title, "")
        }
    }

    pub fn shortcut(self, shortcut: &str) -> Self {
        Self {
            shortcut: Some(shortcut.to_owned()),
            ..self
        }
    }

    pub fn enabled(self, enabled: bool) -> Self {
        Self { enabled, ..self }
    }

    pub fn checked(self, checked: bool) -> Self {
        Self { checked, ..self }
    }

    fn selectable(&self) -> bool {
        self.enabled && !self.separator
    }
}

struct Row {
    top: f32,
    height: f32,
    labels: Option<[Paragraph; 2]>,
    shortcuts: Option<[Paragraph; 2]>,
}

pub struct MenuPanel {
    pub menu: Menu,
    rows: Vec<Row>,
    size: Size,
    highlighted: Option<usize>,
}

impl MenuPanel {
    pub fn new(menu: Menu, text: &mut TextSystem, palette: &Palette) -> Self {
        let on_accent = Color::hex(0xFFFFFF);
        let paragraphs = |text: &mut TextSystem, content: &str, normal: Color| {
            [normal, on_accent].map(|color| {
                text.paragraph(
                    content,
                    TextStyle::new(FONT_SIZE, color),
                    None,
                    TextAlign::Start,
                )
            })
        };
        let mut top = VERTICAL_PADDING;
        let mut widest_label: f32 = 0.0;
        let mut widest_shortcut: f32 = 0.0;
        let has_submenu = menu.items.iter().any(|item| item.submenu.is_some());
        let rows = menu
            .items
            .iter()
            .map(|item| {
                let height = if item.separator {
                    SEPARATOR_HEIGHT
                } else {
                    ROW_HEIGHT
                };
                let label_color = if item.enabled {
                    palette.label_primary
                } else {
                    palette.label_muted
                };
                let labels = (!item.separator).then(|| paragraphs(text, &item.title, label_color));
                let shortcuts = item
                    .shortcut
                    .as_deref()
                    .map(|shortcut| paragraphs(text, shortcut, palette.label_muted));
                if let Some([label, _]) = &labels {
                    widest_label = widest_label.max(label.width());
                }
                if let Some([shortcut, _]) = &shortcuts {
                    widest_shortcut = widest_shortcut.max(shortcut.width());
                }
                let row = Row {
                    top,
                    height,
                    labels,
                    shortcuts,
                };
                top += height;
                row
            })
            .collect();
        let shortcut_space = if widest_shortcut > 0.0 {
            SHORTCUT_GAP + widest_shortcut
        } else {
            0.0
        };
        let chevron_space = if has_submenu { CHEVRON_SPACE } else { 0.0 };
        let width = (TEXT_START + widest_label + shortcut_space + chevron_space + TRAILING_PADDING)
            .max(MINIMUM_WIDTH)
            .ceil();
        Self {
            menu,
            rows,
            size: Size::new(width, top + VERTICAL_PADDING),
            highlighted: None,
        }
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    pub fn row_rect(&self, index: usize) -> Rect {
        self.rows.get(index).map_or(Rect::default(), |row| {
            Rect::new(0.0, row.top, self.size.width, row.height)
        })
    }

    pub fn item_at(&self, point: Point) -> Option<usize> {
        if point.x < 0.0 || point.x >= self.size.width {
            return None;
        }
        self.rows
            .iter()
            .position(|row| point.y >= row.top && point.y < row.top + row.height)
            .filter(|index| self.menu.items[*index].selectable())
    }

    fn step(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let count = self.menu.items.len();
        let selectable = |index: &usize| self.menu.items[*index].selectable();
        match (from, forward) {
            (None, true) => (0..count).find(selectable),
            (None, false) => (0..count).rev().find(selectable),
            (Some(index), true) => (index + 1..count).find(selectable).or(Some(index)),
            (Some(index), false) => (0..index).rev().find(selectable).or(Some(index)),
        }
    }

    pub fn paint(&self, palette: &Palette, show_highlight: bool) -> Vec<Item> {
        let outline = Shape::rounded(
            Rect::new(0.0, 0.0, self.size.width, self.size.height),
            RADIUS,
        );
        let mut items = vec![
            Item::Fill {
                shape: outline,
                paint: palette.surface_elevated.into(),
            },
            Item::Stroke {
                shape: Shape::rounded(outline.rect.inset(0.25), RADIUS - 0.25),
                width: 0.5,
                paint: palette.separator_color.into(),
            },
        ];
        for (index, (item, row)) in self.menu.items.iter().zip(&self.rows).enumerate() {
            let rect = Rect::new(0.0, row.top, self.size.width, row.height);
            let centre = rect.y + rect.height / 2.0;
            if item.separator {
                items.push(Item::Fill {
                    shape: Shape::rect(Rect::new(
                        HIGHLIGHT_INSET * 2.0,
                        centre.round() - 0.5,
                        self.size.width - HIGHLIGHT_INSET * 4.0,
                        1.0,
                    )),
                    paint: palette.separator_color.into(),
                });
                continue;
            }
            let lit = show_highlight && self.highlighted == Some(index);
            if lit {
                items.push(Item::Fill {
                    shape: Shape::rounded(
                        Rect::new(
                            HIGHLIGHT_INSET,
                            rect.y,
                            self.size.width - 2.0 * HIGHLIGHT_INSET,
                            rect.height,
                        ),
                        4.0,
                    ),
                    paint: palette.accent_color.into(),
                });
            }
            let variant = usize::from(lit);
            let mark = if lit {
                Color::hex(0xFFFFFF)
            } else if item.enabled {
                palette.label_primary
            } else {
                palette.label_muted
            };
            if let Some(labels) = &row.labels {
                let label = &labels[variant];
                items.push(Item::Text {
                    origin: Point::new(TEXT_START, (centre - label.height() / 2.0).round()),
                    paragraph: label.clone(),
                });
            }
            if let Some(shortcuts) = &row.shortcuts {
                let shortcut = &shortcuts[variant];
                let right = self.size.width - TRAILING_PADDING;
                items.push(Item::Text {
                    origin: Point::new(
                        (right - shortcut.width()).round(),
                        (centre - shortcut.height() / 2.0).round(),
                    ),
                    paragraph: shortcut.clone(),
                });
            }
            let line = |from: (f32, f32), to: (f32, f32), width: f32| Item::Line {
                from: Point::new(from.0, centre + from.1),
                to: Point::new(to.0, centre + to.1),
                width,
                paint: mark.into(),
            };
            if item.checked {
                items.push(line((9.0, 0.5), (12.0, 3.5), 1.5));
                items.push(line((12.0, 3.5), (17.0, -3.5), 1.5));
            }
            if item.submenu.is_some() {
                let x = self.size.width - TRAILING_PADDING;
                items.push(line((x - 3.0, -4.0), (x + 1.0, 0.0), 1.4));
                items.push(line((x + 1.0, 0.0), (x - 3.0, 4.0), 1.4));
            }
        }
        items
    }

    fn access(&self, level: usize) -> Vec<(NodeId, Node)> {
        let id = |index: usize| menu_node(level, Some(index));
        let mut menu = Node::new(Role::Menu);
        menu.set_children(
            (0..self.menu.items.len())
                .filter(|index| !self.menu.items[*index].separator)
                .map(id)
                .collect::<Vec<_>>(),
        );
        let mut nodes = vec![(menu_node(level, None), menu)];
        for (index, item) in self.menu.items.iter().enumerate() {
            if item.separator {
                continue;
            }
            let role = if item.checked {
                Role::MenuItemCheckBox
            } else {
                Role::MenuItem
            };
            let mut node = Node::new(role);
            node.set_label(item.title.clone());
            if item.checked {
                node.set_toggled(Toggled::True);
            }
            if let Some(shortcut) = &item.shortcut {
                node.set_keyboard_shortcut(shortcut.clone());
            }
            if item.enabled {
                node.add_action(Action::Click);
            } else {
                node.set_disabled();
            }
            if item.submenu.is_some() {
                node.set_has_popup(accesskit::HasPopup::Menu);
            }
            let rect = self.row_rect(index);
            node.set_bounds(accesskit::Rect {
                x0: f64::from(rect.x),
                y0: f64::from(rect.y),
                x1: f64::from(rect.x + rect.width),
                y1: f64::from(rect.y + rect.height),
            });
            nodes.push((id(index), node));
        }
        nodes
    }
}

const MENU_NODES: u64 = u64::MAX - 1024;

fn menu_node(level: usize, index: Option<usize>) -> NodeId {
    let level = level as u64 * 256;
    NodeId(MENU_NODES - level - index.map_or(0, |index| index as u64 + 1))
}

#[derive(Debug, Clone, PartialEq)]
pub enum MenuOutcome {
    Nothing,
    Redraw,
    OpenSubmenu { level: usize, index: usize },
    CloseFrom(usize),
    Activate(String),
    Dismiss,
}

pub struct MenuTracker {
    panels: Vec<MenuPanel>,
    typed: String,
    typed_at: Option<Instant>,
    submenu_due: Option<(usize, usize, Instant)>,
    armed: bool,
    flash: Option<(usize, Instant)>,
}

impl MenuTracker {
    pub fn new(root: MenuPanel) -> Self {
        Self {
            panels: vec![root],
            typed: String::new(),
            typed_at: None,
            submenu_due: None,
            armed: false,
            flash: None,
        }
    }

    pub fn panels(&self) -> &[MenuPanel] {
        &self.panels
    }

    pub fn truncate(&mut self, levels: usize) {
        self.panels.truncate(levels.max(1));
    }

    pub fn submenu(&self, level: usize, index: usize) -> Option<Menu> {
        self.panels
            .get(level)?
            .menu
            .items
            .get(index)?
            .submenu
            .clone()
    }

    fn highlight(&mut self, level: usize, index: Option<usize>, now: Instant) -> MenuOutcome {
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        if panel.highlighted == index {
            return MenuOutcome::Nothing;
        }
        panel.highlighted = index;
        self.submenu_due = index
            .filter(|index| panel.menu.items[*index].submenu.is_some())
            .map(|index| (level, index, now + SUBMENU_DELAY));
        if self.panels.len() > level + 1 {
            return MenuOutcome::CloseFrom(level + 1);
        }
        MenuOutcome::Redraw
    }

    pub fn pointer_move(&mut self, level: usize, point: Point, now: Instant) -> MenuOutcome {
        if self.flash.is_some() {
            return MenuOutcome::Nothing;
        }
        let Some(panel) = self.panels.get(level) else {
            return MenuOutcome::Nothing;
        };
        let index = panel.item_at(point);
        if index.is_some() {
            self.armed = true;
        }
        let keeps_submenu = index.is_none() && self.panels.len() > level + 1;
        if keeps_submenu {
            return MenuOutcome::Nothing;
        }
        self.highlight(level, index, now)
    }

    pub fn pointer_press(&mut self, level: usize, point: Point) -> MenuOutcome {
        let Some(panel) = self.panels.get(level) else {
            return MenuOutcome::Nothing;
        };
        self.armed = true;
        match panel.item_at(point) {
            Some(index) if panel.menu.items[index].submenu.is_some() => {
                self.open_submenu(level, index)
            }
            _ => MenuOutcome::Nothing,
        }
    }

    pub fn pointer_release(&mut self, level: usize, point: Point, now: Instant) -> MenuOutcome {
        if !self.armed || self.flash.is_some() {
            return MenuOutcome::Nothing;
        }
        let Some(index) = self
            .panels
            .get(level)
            .and_then(|panel| panel.item_at(point))
        else {
            return MenuOutcome::Nothing;
        };
        self.activate(level, index, now)
    }

    fn open_submenu(&mut self, level: usize, index: usize) -> MenuOutcome {
        self.submenu_due = None;
        if self.panels.len() > level + 1 {
            return MenuOutcome::Nothing;
        }
        MenuOutcome::OpenSubmenu { level, index }
    }

    pub fn activate(&mut self, level: usize, index: usize, now: Instant) -> MenuOutcome {
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        let Some(item) = panel.menu.items.get(index).filter(|item| item.selectable()) else {
            return MenuOutcome::Nothing;
        };
        if item.submenu.is_some() {
            return self.open_submenu(level, index);
        }
        let action = item.action.clone();
        panel.highlighted = Some(index);
        self.flash = Some((level, now));
        self.submenu_due = None;
        MenuOutcome::Activate(action)
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers, now: Instant) -> MenuOutcome {
        if self.flash.is_some() {
            return MenuOutcome::Nothing;
        }
        let level = self.panels.len() - 1;
        let current = self.panels[level].highlighted;
        match key {
            Key::Up | Key::Down => {
                let next = self.panels[level].step(current, key == Key::Down);
                self.highlight(level, next, now)
            }
            Key::Home | Key::End => {
                let next = self.panels[level].step(None, key == Key::Home);
                self.highlight(level, next, now)
            }
            Key::Right => match current {
                Some(index) if self.panels[level].menu.items[index].submenu.is_some() => {
                    self.open_submenu(level, index)
                }
                _ => MenuOutcome::Nothing,
            },
            Key::Left if level > 0 => MenuOutcome::CloseFrom(level),
            Key::Escape if level > 0 => MenuOutcome::CloseFrom(level),
            Key::Escape => MenuOutcome::Dismiss,
            Key::Enter | Key::Space => match current {
                Some(index) => self.activate(level, index, now),
                None => MenuOutcome::Nothing,
            },
            Key::Character(character) if !modifiers.logo && !modifiers.control => {
                self.type_select(level, character, now)
            }
            _ => MenuOutcome::Nothing,
        }
    }

    fn type_select(&mut self, level: usize, character: char, now: Instant) -> MenuOutcome {
        let fresh = self
            .typed_at
            .is_none_or(|at| now.duration_since(at) > TYPE_SELECT_PAUSE);
        if fresh {
            self.typed.clear();
        }
        self.typed.extend(character.to_lowercase());
        self.typed_at = Some(now);
        let typed = self.typed.clone();
        let found =
            self.panels[level].menu.items.iter().position(|item| {
                item.selectable() && item.title.to_lowercase().starts_with(&typed)
            });
        match found {
            Some(index) => self.highlight(level, Some(index), now),
            None => MenuOutcome::Nothing,
        }
    }

    pub fn opened_submenu(&mut self, panel: MenuPanel, from_keyboard: bool) {
        let mut panel = panel;
        if from_keyboard {
            panel.highlighted = panel.step(None, true);
        }
        self.panels.push(panel);
    }

    pub fn deadline(&self) -> Option<Instant> {
        let submenu = self.submenu_due.map(|(_, _, at)| at);
        let flash = self.flash.map(|(_, at)| at + 2 * FLASH_STEP + FADE);
        submenu.into_iter().chain(flash).min()
    }

    pub fn tick(&mut self, now: Instant) -> MenuOutcome {
        if let Some((level, index, at)) = self.submenu_due
            && at <= now
        {
            return self.open_submenu(level, index);
        }
        if self.is_finished(now) {
            return MenuOutcome::Dismiss;
        }
        MenuOutcome::Nothing
    }

    pub fn is_finished(&self, now: Instant) -> bool {
        self.flash
            .is_some_and(|(_, at)| now >= at + 2 * FLASH_STEP + FADE)
    }

    pub fn is_flashing(&self) -> bool {
        self.flash.is_some()
    }

    pub fn appearance(&self, level: usize, now: Instant) -> (bool, f32) {
        let Some((flashed, at)) = self.flash else {
            return (true, 1.0);
        };
        let elapsed = now.saturating_duration_since(at);
        let blink_off = elapsed < FLASH_STEP && level == flashed;
        let fade = elapsed.saturating_sub(2 * FLASH_STEP).as_secs_f32() / FADE.as_secs_f32();
        (!blink_off, (1.0 - fade).clamp(0.0, 1.0))
    }

    pub fn access(&self) -> (Vec<(NodeId, Node)>, Vec<NodeId>, Option<NodeId>) {
        let mut nodes = Vec::new();
        let mut roots = Vec::new();
        for (level, panel) in self.panels.iter().enumerate() {
            roots.push(menu_node(level, None));
            nodes.extend(panel.access(level));
        }
        let level = self.panels.len() - 1;
        let focus = self.panels[level]
            .highlighted
            .map(|index| menu_node(level, Some(index)))
            .or(Some(menu_node(level, None)));
        (nodes, roots, focus)
    }

    pub fn item_for_node(&self, node: NodeId) -> Option<(usize, usize)> {
        self.panels.iter().enumerate().find_map(|(level, panel)| {
            (0..panel.menu.items.len())
                .find(|index| menu_node(level, Some(*index)) == node)
                .map(|index| (level, index))
        })
    }
}
