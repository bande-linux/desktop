use std::time::{Duration, Instant};

use accesskit::{Action, Node, NodeId, Role, Toggled};
use unbound_graphics::{
    Color, Item, Palette, Paragraph, Point, Rect, Shape, TextAlign, TextStyle, TextSystem,
};

use super::{Key, Modifiers, Size};

pub(crate) const ROW_HEIGHT: f32 = 22.0;
const SEPARATOR_HEIGHT: f32 = 11.0;
pub(crate) const VERTICAL_PADDING: f32 = 5.0;
const HIGHLIGHT_INSET: f32 = 5.0;
pub(crate) const TEXT_START: f32 = 24.0;
const TRAILING_PADDING: f32 = 16.0;
const SHORTCUT_GAP: f32 = 28.0;
const CHEVRON_SPACE: f32 = 18.0;
const MINIMUM_WIDTH: f32 = 120.0;
const RADIUS: f32 = 8.0;
const FONT_SIZE: f32 = 13.0;
pub const SUBMENU_DELAY: Duration = Duration::from_millis(200);
const SAFE_DELAY: Duration = Duration::from_millis(300);
const SAFE_MARGIN: f32 = 6.0;
const FIELD_HEIGHT: f32 = 30.0;
const FIELD_INSET: f32 = 6.0;
const FIELD_RADIUS: f32 = 6.0;
const FIELD_ALPHA: u8 = 16;
const SEARCH_WIDTH: f32 = 260.0;
const MAX_FOUND: usize = 12;
const PATH_SEPARATOR: &str = " › ";
const TYPE_SELECT_PAUSE: Duration = Duration::from_secs(1);
const FLASH_STEP: Duration = Duration::from_millis(70);
const FADE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub search: Option<Search>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Search {
    pub placeholder: String,
    pub nothing_found: String,
    pub query: String,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            search: None,
        }
    }

    pub fn searchable(self, placeholder: &str, nothing_found: &str) -> Self {
        Self {
            search: Some(Search {
                placeholder: placeholder.to_owned(),
                nothing_found: nothing_found.to_owned(),
                query: String::new(),
            }),
            ..self
        }
    }

    pub fn found(&self, query: &str) -> Menu {
        let query = query.to_lowercase();
        let mut items = Vec::new();
        collect_found(&self.items, &[], &query, &mut items);
        let search = self.search.clone().map(|search| Search {
            query: query.clone(),
            ..search
        });
        if items.is_empty() {
            let nothing = search
                .as_ref()
                .map(|search| search.nothing_found.clone())
                .unwrap_or_default();
            items.push(MenuItem::new(&nothing, "").enabled(false));
        }
        items.truncate(MAX_FOUND);
        Menu { items, search }
    }
}

fn collect_found(items: &[MenuItem], path: &[&str], query: &str, found: &mut Vec<MenuItem>) {
    for item in items {
        if item.separator {
            continue;
        }
        match &item.submenu {
            Some(submenu) if item.enabled => {
                let mut inner = path.to_vec();
                inner.push(&item.title);
                collect_found(&submenu.items, &inner, query, found);
            }
            Some(_) => {}
            None if item.enabled && item.title.to_lowercase().contains(query) => {
                let mut title = path.join(PATH_SEPARATOR);
                if !title.is_empty() {
                    title.push_str(PATH_SEPARATOR);
                }
                title.push_str(&item.title);
                found.push(MenuItem {
                    title,
                    alternate: None,
                    ..item.clone()
                });
            }
            None => {}
        }
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
    pub alternate: Option<Box<MenuItem>>,
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
            alternate: None,
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

    pub fn alternate(self, alternate: MenuItem) -> Self {
        Self {
            alternate: Some(Box::new(alternate)),
            ..self
        }
    }

    fn selectable(&self) -> bool {
        self.enabled && !self.separator
    }
}

const MODIFIER_SYMBOLS: [(&[&str], char); 4] = [
    (&["Ctrl", "Control"], '⌃'),
    (&["Alt", "Option"], '⌥'),
    (&["Shift"], '⇧'),
    (&["Super", "Logo", "Meta"], '⌘'),
];

const KEY_SYMBOLS: [(&str, &str); 14] = [
    ("Backspace", "⌫"),
    ("Delete", "⌦"),
    ("Return", "↩"),
    ("Enter", "↩"),
    ("Escape", "⎋"),
    ("Tab", "⇥"),
    ("Left", "←"),
    ("Right", "→"),
    ("Up", "↑"),
    ("Down", "↓"),
    ("PageUp", "⇞"),
    ("PageDown", "⇟"),
    ("Home", "↖"),
    ("End", "↘"),
];

pub fn shortcut_symbols(shortcut: &str) -> String {
    let parts: Vec<&str> = shortcut.split('+').collect();
    let (key, modifiers) = match parts.split_last() {
        Some((&"", rest)) if rest.last() == Some(&"") => ("+", &rest[..rest.len() - 1]),
        Some((key, rest)) => (*key, rest),
        None => ("", &[][..]),
    };
    let mut symbols: String = MODIFIER_SYMBOLS
        .iter()
        .filter(|(names, _)| {
            modifiers
                .iter()
                .any(|modifier| names.iter().any(|name| name.eq_ignore_ascii_case(modifier)))
        })
        .map(|(_, symbol)| *symbol)
        .collect();
    match KEY_SYMBOLS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
    {
        Some((_, symbol)) => symbols.push_str(symbol),
        None if key.chars().count() == 1 => {
            symbols.extend(key.chars().flat_map(char::to_uppercase))
        }
        None => symbols.push_str(key),
    }
    symbols
}

struct Row {
    top: f32,
    height: f32,
    labels: Option<[Paragraph; 2]>,
    shortcuts: Option<[Paragraph; 2]>,
}

pub struct MenuPanel {
    menus: [Menu; 2],
    rows: [Vec<Row>; 2],
    alternates: bool,
    size: Size,
    highlighted: Option<usize>,
    field: Option<(Paragraph, bool)>,
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
        let alternate = Menu::new(
            menu.items
                .iter()
                .map(|item| {
                    item.alternate
                        .as_deref()
                        .cloned()
                        .unwrap_or_else(|| item.clone())
                })
                .collect(),
        );
        let mut widest_label: f32 = 0.0;
        let mut widest_shortcut: f32 = 0.0;
        let has_submenu = menu.items.iter().any(|item| item.submenu.is_some());
        let field = menu.search.as_ref().map(|search| {
            let (content, color) = if search.query.is_empty() {
                (search.placeholder.as_str(), palette.ink2)
            } else {
                (search.query.as_str(), palette.ink)
            };
            (
                text.paragraph(
                    content,
                    TextStyle::new(FONT_SIZE, color),
                    None,
                    TextAlign::Start,
                ),
                !search.query.is_empty(),
            )
        });
        let field_height = if field.is_some() { FIELD_HEIGHT } else { 0.0 };
        let minimum_width = if field.is_some() {
            SEARCH_WIDTH
        } else {
            MINIMUM_WIDTH
        };
        let rows = [&menu, &alternate].map(|menu| {
            let mut top = VERTICAL_PADDING + field_height;
            menu.items
                .iter()
                .map(|item| {
                    let height = if item.separator {
                        SEPARATOR_HEIGHT
                    } else {
                        ROW_HEIGHT
                    };
                    let label_color = if item.enabled {
                        palette.ink
                    } else {
                        palette.ink2
                    };
                    let labels =
                        (!item.separator).then(|| paragraphs(text, &item.title, label_color));
                    let shortcuts = item.shortcut.as_deref().map(|shortcut| {
                        paragraphs(text, &shortcut_symbols(shortcut), palette.ink2)
                    });
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
                .collect::<Vec<_>>()
        });
        let top = rows[0]
            .last()
            .map_or(VERTICAL_PADDING + field_height, |row| row.top + row.height);
        let shortcut_space = if widest_shortcut > 0.0 {
            SHORTCUT_GAP + widest_shortcut
        } else {
            0.0
        };
        let chevron_space = if has_submenu { CHEVRON_SPACE } else { 0.0 };
        let width = (TEXT_START + widest_label + shortcut_space + chevron_space + TRAILING_PADDING)
            .max(minimum_width)
            .ceil();
        Self {
            menus: [menu, alternate],
            rows,
            alternates: false,
            size: Size::new(width, top + VERTICAL_PADDING),
            highlighted: None,
            field,
        }
    }

    pub fn at_least(self, size: Size) -> Self {
        Self {
            size: Size::new(
                self.size.width.max(size.width),
                self.size.height.max(size.height),
            ),
            ..self
        }
    }

    pub fn shows_alternates(&self) -> bool {
        self.alternates
    }

    pub fn menu(&self) -> &Menu {
        &self.menus[usize::from(self.alternates)]
    }

    fn current_rows(&self) -> &[Row] {
        &self.rows[usize::from(self.alternates)]
    }

    pub fn size(&self) -> Size {
        self.size
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    pub fn row_rect(&self, index: usize) -> Rect {
        self.current_rows()
            .get(index)
            .map_or(Rect::default(), |row| {
                Rect::new(0.0, row.top, self.size.width, row.height)
            })
    }

    pub fn item_at(&self, point: Point) -> Option<usize> {
        if point.x < 0.0 || point.x >= self.size.width {
            return None;
        }
        self.current_rows()
            .iter()
            .position(|row| point.y >= row.top && point.y < row.top + row.height)
            .filter(|index| self.menu().items[*index].selectable())
    }

    fn step(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let count = self.menu().items.len();
        let selectable = |index: &usize| self.menu().items[*index].selectable();
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
                paint: palette.overlay.into(),
            },
            Item::Stroke {
                shape: Shape::rounded(outline.rect.inset(0.25), RADIUS - 0.25),
                width: 0.5,
                paint: palette.hairline.into(),
            },
        ];
        if let Some((field, typed)) = &self.field {
            let well = Rect::new(
                FIELD_INSET,
                VERTICAL_PADDING,
                self.size.width - 2.0 * FIELD_INSET,
                FIELD_HEIGHT - FIELD_INSET,
            );
            items.push(Item::Fill {
                shape: Shape::rounded(well, FIELD_RADIUS),
                paint: palette.ink.with_alpha(FIELD_ALPHA).into(),
            });
            let x = well.x + 2.0 * FIELD_INSET;
            let y = (well.y + (well.height - field.height()) / 2.0).round();
            items.push(Item::Text {
                origin: Point::new(x, y),
                paragraph: field.clone(),
            });
            if *typed {
                let caret = (x + field.width() + 1.0).round() + 0.5;
                items.push(Item::Line {
                    from: Point::new(caret, y + 1.0),
                    to: Point::new(caret, y + field.height() - 1.0),
                    width: 1.0,
                    paint: palette.accent.into(),
                });
            }
        }
        for (index, (item, row)) in self
            .menu()
            .items
            .iter()
            .zip(self.current_rows())
            .enumerate()
        {
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
                    paint: palette.hairline.into(),
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
                    paint: palette.accent.into(),
                });
            }
            let variant = usize::from(lit);
            let mark = if lit {
                Color::hex(0xFFFFFF)
            } else if item.enabled {
                palette.ink
            } else {
                palette.ink2
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
            (0..self.menu().items.len())
                .filter(|index| !self.menu().items[*index].separator)
                .map(id)
                .collect::<Vec<_>>(),
        );
        let mut nodes = vec![(menu_node(level, None), menu)];
        for (index, item) in self.menu().items.iter().enumerate() {
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
    Search(Menu),
}

fn entered(mut panel: MenuPanel, from_keyboard: bool) -> MenuPanel {
    if from_keyboard {
        panel.highlighted = panel.step(None, true);
    }
    panel
}

pub struct MenuTracker {
    panels: Vec<MenuPanel>,
    typed: String,
    typed_at: Option<Instant>,
    submenu_due: Option<(usize, usize, Instant)>,
    armed: bool,
    flash: Option<(usize, Instant)>,
    searched: Option<(Menu, String)>,
    revision: u32,
    pointer: Option<(usize, Point)>,
    settle: Option<(usize, Point, Instant)>,
}

fn inside_triangle(point: Point, corners: [Point; 3]) -> bool {
    let side = |a: Point, b: Point| (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x);
    let [a, b, c] = corners;
    let sides = [side(a, b), side(b, c), side(c, a)];
    sides.iter().all(|value| *value >= 0.0) || sides.iter().all(|value| *value <= 0.0)
}

impl MenuTracker {
    pub fn new(root: MenuPanel, from_keyboard: bool) -> Self {
        let searched = root
            .menu()
            .search
            .is_some()
            .then(|| (root.menu().clone(), String::new()));
        Self {
            panels: vec![entered(root, from_keyboard)],
            typed: String::new(),
            typed_at: None,
            submenu_due: None,
            armed: false,
            flash: None,
            searched,
            revision: 0,
            pointer: None,
            settle: None,
        }
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn searched(&mut self, panel: MenuPanel) {
        let size = self.panels[0].size();
        let mut panel = panel.at_least(size);
        panel.highlighted = panel.step(None, true);
        self.panels = vec![panel];
        self.submenu_due = None;
        self.settle = None;
        self.revision = self.revision.wrapping_add(1);
    }

    fn search(&mut self, edit: impl FnOnce(&mut String)) -> MenuOutcome {
        let Some((source, query)) = &mut self.searched else {
            return MenuOutcome::Nothing;
        };
        let before = query.clone();
        edit(query);
        if *query == before {
            return MenuOutcome::Nothing;
        }
        let menu = if query.is_empty() {
            source.clone()
        } else {
            source.found(query)
        };
        MenuOutcome::Search(menu)
    }

    fn searching(&self) -> bool {
        self.searched
            .as_ref()
            .is_some_and(|(_, query)| !query.is_empty())
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
            .menu()
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
            .filter(|index| panel.menu().items[*index].submenu.is_some())
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
        let from = self
            .pointer
            .filter(|(known, _)| *known == level)
            .map(|(_, from)| from);
        self.pointer = Some((level, point));
        let keeps_submenu = index.is_none() && self.panels.len() > level + 1;
        if keeps_submenu {
            return MenuOutcome::Nothing;
        }
        if let (Some(from), Some(open)) = (from, self.heading_to_submenu(level))
            && index != Some(open)
            && self.toward_submenu(level, open, from, point)
        {
            self.settle = Some((level, point, now + SAFE_DELAY));
            return MenuOutcome::Nothing;
        }
        self.settle = None;
        self.highlight(level, index, now)
    }

    fn heading_to_submenu(&self, level: usize) -> Option<usize> {
        let open = self.panels.get(level)?.highlighted?;
        let opening = self
            .submenu_due
            .is_some_and(|(due, index, _)| due == level && index == open);
        (self.panels.len() > level + 1 || opening).then_some(open)
    }

    fn toward_submenu(&self, level: usize, open: usize, from: Point, point: Point) -> bool {
        let panel = &self.panels[level];
        let row = panel.row_rect(open);
        let height = self
            .panels
            .get(level + 1)
            .map_or(row.height, |submenu| submenu.size().height);
        let edge = panel.size().width;
        point.x > from.x
            && inside_triangle(
                point,
                [
                    Point::new(from.x - 1.0, from.y),
                    Point::new(edge + SAFE_MARGIN, row.y - SAFE_MARGIN),
                    Point::new(edge + SAFE_MARGIN, row.y + height + SAFE_MARGIN),
                ],
            )
    }

    pub fn pointer_press(&mut self, level: usize, point: Point) -> MenuOutcome {
        let Some(panel) = self.panels.get(level) else {
            return MenuOutcome::Nothing;
        };
        self.armed = true;
        match panel.item_at(point) {
            Some(index) if panel.menu().items[index].submenu.is_some() => {
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
        let Some(item) = panel
            .menu()
            .items
            .get(index)
            .filter(|item| item.selectable())
        else {
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
                Some(index) if self.panels[level].menu().items[index].submenu.is_some() => {
                    self.open_submenu(level, index)
                }
                _ => MenuOutcome::Nothing,
            },
            Key::Left if level > 0 => MenuOutcome::CloseFrom(level),
            Key::Escape if level > 0 => MenuOutcome::CloseFrom(level),
            Key::Escape if self.searching() => self.search(String::clear),
            Key::Escape => MenuOutcome::Dismiss,
            Key::Backspace if self.searching() => self.search(|query| {
                query.pop();
            }),
            Key::Enter | Key::Space => match current {
                Some(index) => self.activate(level, index, now),
                None => MenuOutcome::Nothing,
            },
            Key::Character(character)
                if !modifiers.logo && !modifiers.control && self.searched.is_some() =>
            {
                self.search(|query| query.push(character))
            }
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
            self.panels[level].menu().items.iter().position(|item| {
                item.selectable() && item.title.to_lowercase().starts_with(&typed)
            });
        match found {
            Some(index) => self.highlight(level, Some(index), now),
            None => MenuOutcome::Nothing,
        }
    }

    pub fn show_alternates(&mut self, shown: bool) -> MenuOutcome {
        let mut changed = false;
        for panel in &mut self.panels {
            changed |= panel.alternates != shown;
            panel.alternates = shown;
        }
        match changed {
            true => MenuOutcome::Redraw,
            false => MenuOutcome::Nothing,
        }
    }

    pub fn opened_submenu(&mut self, panel: MenuPanel, from_keyboard: bool) {
        let mut panel = entered(panel, from_keyboard);
        panel.alternates = self.panels.first().is_some_and(|root| root.alternates);
        self.panels.push(panel);
    }

    pub fn deadline(&self) -> Option<Instant> {
        let submenu = self.submenu_due.map(|(_, _, at)| at);
        let flash = self.flash.map(|(_, at)| at + 2 * FLASH_STEP + FADE);
        let settle = self.settle.map(|(_, _, at)| at);
        submenu.into_iter().chain(flash).chain(settle).min()
    }

    pub fn tick(&mut self, now: Instant) -> MenuOutcome {
        if let Some((level, point, at)) = self.settle
            && at <= now
        {
            self.settle = None;
            let index = self
                .panels
                .get(level)
                .and_then(|panel| panel.item_at(point));
            return self.highlight(level, index, now);
        }
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
            (0..panel.menu().items.len())
                .find(|index| menu_node(level, Some(*index)) == node)
                .map(|index| (level, index))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::shortcut_symbols;

    #[test]
    fn shortcuts_read_as_key_symbols_in_the_standard_modifier_order() {
        assert_eq!(shortcut_symbols("Super+Q"), "⌘Q");
        assert_eq!(shortcut_symbols("Shift+Super+N"), "⇧⌘N");
        assert_eq!(shortcut_symbols("Super+Ctrl+Alt+Shift+z"), "⌃⌥⇧⌘Z");
        assert_eq!(shortcut_symbols("Control+Shift+q"), "⌃⇧Q");
        assert_eq!(shortcut_symbols("Alt+Super+Backspace"), "⌥⌘⌫");
        assert_eq!(shortcut_symbols("Ctrl+Super+Left"), "⌃⌘←");
        assert_eq!(shortcut_symbols("Super++"), "⌘+");
        assert_eq!(shortcut_symbols("Shift+Super+."), "⇧⌘.");
        assert_eq!(shortcut_symbols("F2"), "F2");
        assert_eq!(shortcut_symbols("Space"), "Space");
    }
}
