use std::time::{Duration, Instant};

use accesskit::{Action, Node, NodeId, Role, Toggled};
use unbound_graphics::{
    Color, Item, Palette, Paragraph, Path, Point, Radius, Rect, Shape, TextAlign, TextStyle,
    TextSystem,
};

use super::control::square_capped;
use super::shortcut::ShortcutText;
use super::{Animated, Key, Modifiers, Size, Spring};

pub(crate) const ROW_HEIGHT: f32 = 28.0;
const SEPARATOR_HEIGHT: f32 = 9.0;
pub(crate) const VERTICAL_PADDING: f32 = 4.0;
const HIGHLIGHT_INSET: f32 = 4.0;
const SEPARATOR_INSET: f32 = 12.0;
pub(crate) const TEXT_START: f32 = 28.0;
const TRAILING_PADDING: f32 = 12.0;
const SHORTCUT_GAP: f32 = 28.0;
const CHEVRON_SPACE: f32 = 18.0;
const MINIMUM_WIDTH: f32 = 140.0;
pub const MENU_RADIUS: f32 = Radius::L;
const FONT_SIZE: f32 = 13.0;
const MARK_STROKE: f32 = 1.6;
pub const SUBMENU_DELAY: Duration = Duration::from_millis(200);
const SAFE_DELAY: Duration = Duration::from_millis(300);
const SAFE_MARGIN: f32 = 6.0;
const SEARCH_LINE: f32 = ROW_HEIGHT + SEPARATOR_HEIGHT;
const FOOTER_HEIGHT: f32 = 28.0;
const FOOTER_FONT_SIZE: f32 = 12.0;
const SEARCH_WIDTH: f32 = 220.0;
const MAGNIFIER_RADIUS: f32 = 4.0;
const MAGNIFIER_CENTRE: f32 = 14.0;
const MAX_FOUND: usize = 12;
const PATH_SEPARATOR: &str = " › ";
const TYPE_SELECT_PAUSE: Duration = Duration::from_secs(1);
const FLASH_STEP: Duration = Duration::from_millis(50);
const FADE_LIMIT: Duration = Duration::from_millis(500);
const SCROLL_ZONE: f32 = 20.0;
const FEWEST_ROWS: f32 = 3.0;
const SCROLL_SPEED: f32 = 300.0;
const SCROLL_STEP: Duration = Duration::from_millis(16);
const CHEVRON_HALF_WIDTH: f32 = 4.0;
const CHEVRON_RISE: f32 = 2.0;

fn fade_after(chosen: Instant) -> Animated {
    let mut fade = Animated::new(1.0);
    fade.animate_at(chosen + 2 * FLASH_STEP, false, 0.0, Spring::FADE);
    fade
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    pub items: Vec<MenuItem>,
    pub search: Option<Search>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Search {
    pub hint: String,
    pub shortcut: String,
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

    pub fn searchable(self, hint: &str, shortcut: &str, nothing_found: &str) -> Self {
        Self {
            search: Some(Search {
                hint: hint.to_owned(),
                shortcut: shortcut.to_owned(),
                nothing_found: nothing_found.to_owned(),
                query: String::new(),
            }),
            ..self
        }
    }

    pub fn matching(&self, query: &str) -> Vec<MenuItem> {
        let mut items = Vec::new();
        collect_found(&self.items, &[], &query.to_lowercase(), &mut items);
        items
    }

    pub fn found(&self, query: &str) -> Menu {
        let query = query.to_lowercase();
        let mut items = self.matching(&query);
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
                    strong: false,
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
    pub strong: bool,
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
            strong: false,
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

    pub fn strong(self) -> Self {
        Self {
            strong: true,
            ..self
        }
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

struct Row {
    top: f32,
    height: f32,
    labels: Option<[Paragraph; 2]>,
    shortcuts: Option<[ShortcutText; 2]>,
}

pub struct MenuPanel {
    menus: [Menu; 2],
    rows: [Vec<Row>; 2],
    alternates: bool,
    size: Size,
    content_height: f32,
    scrolled: f32,
    submenus_open_left: bool,
    highlighted: Option<usize>,
    search: Option<SearchLook>,
}

enum SearchLook {
    Hint(Paragraph, ShortcutText),
    Query(Paragraph),
}

impl MenuPanel {
    pub fn new(menu: Menu, text: &mut TextSystem, palette: &Palette) -> Self {
        let on_accent = palette.on_accent;
        let paragraphs = |text: &mut TextSystem, content: &str, normal: Color, strong: bool| {
            [normal, on_accent].map(|color| {
                let style = TextStyle::new(FONT_SIZE, color);
                let style = if strong {
                    style.weight(TextStyle::BOLD)
                } else {
                    style
                };
                text.paragraph(content, style, None, TextAlign::Start)
            })
        };
        let shortcut_texts = |text: &mut TextSystem, shortcut: &str| {
            [palette.ink3, on_accent]
                .map(|color| ShortcutText::new(text, shortcut, TextStyle::new(FONT_SIZE, color)))
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
        let search = menu.search.as_ref().map(|search| {
            if search.query.is_empty() {
                let style = TextStyle::new(FOOTER_FONT_SIZE, palette.ink3);
                SearchLook::Hint(
                    text.paragraph(&search.hint, style, None, TextAlign::Start),
                    ShortcutText::new(text, &search.shortcut, style),
                )
            } else {
                SearchLook::Query(text.paragraph(
                    &search.query,
                    TextStyle::new(FONT_SIZE, palette.ink),
                    None,
                    TextAlign::Start,
                ))
            }
        });
        let (header, footer, minimum_width) = match &search {
            Some(SearchLook::Hint(hint, shortcut)) => (
                0.0,
                FOOTER_HEIGHT,
                (TEXT_START + hint.width() + SHORTCUT_GAP + shortcut.width() + TRAILING_PADDING)
                    .max(SEARCH_WIDTH),
            ),
            Some(SearchLook::Query(_)) => (SEARCH_LINE, 0.0, SEARCH_WIDTH),
            None => (0.0, 0.0, MINIMUM_WIDTH),
        };
        let rows = [&menu, &alternate].map(|menu| {
            let mut top = VERTICAL_PADDING + header;
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
                        palette.ink3
                    };
                    let labels = (!item.separator)
                        .then(|| paragraphs(text, &item.title, label_color, item.strong));
                    let shortcuts = item
                        .shortcut
                        .as_deref()
                        .map(|shortcut| shortcut_texts(text, shortcut));
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
            .map_or(VERTICAL_PADDING + header, |row| row.top + row.height);
        let shortcut_space = if widest_shortcut > 0.0 {
            SHORTCUT_GAP + widest_shortcut
        } else {
            0.0
        };
        let chevron_space = if has_submenu { CHEVRON_SPACE } else { 0.0 };
        let width = (TEXT_START + widest_label + shortcut_space + chevron_space + TRAILING_PADDING)
            .max(minimum_width)
            .ceil();
        let height = top + VERTICAL_PADDING + footer;
        Self {
            menus: [menu, alternate],
            rows,
            alternates: false,
            size: Size::new(width, height),
            content_height: height,
            scrolled: 0.0,
            submenus_open_left: false,
            highlighted: None,
            search,
        }
    }

    pub fn at_least_as_wide_as(self, width: f32) -> Self {
        Self {
            size: Size::new(self.size.width.max(width), self.size.height),
            ..self
        }
    }

    pub fn fit_height(&mut self, height: f32) {
        let (header, footer) = self.header_and_footer();
        let fewest = 2.0 * VERTICAL_PADDING + header + footer + 2.0 * SCROLL_ZONE;
        let fewest = fewest + FEWEST_ROWS * ROW_HEIGHT;
        self.size.height = height
            .max(fewest.min(self.content_height))
            .min(self.content_height);
        self.scrolled = self.scrolled.clamp(0.0, self.scroll_limit());
        if let Some(index) = self.highlighted {
            self.reveal(index);
        }
    }

    pub fn scroll_offset(&self) -> f32 {
        self.scrolled
    }

    pub fn is_scrollable(&self) -> bool {
        self.scroll_limit() > 0.0
    }

    fn scroll_limit(&self) -> f32 {
        (self.content_height - self.size.height).max(0.0)
    }

    fn header_and_footer(&self) -> (f32, f32) {
        match &self.search {
            Some(SearchLook::Query(_)) => (SEARCH_LINE, 0.0),
            Some(SearchLook::Hint(..)) => (0.0, FOOTER_HEIGHT),
            None => (0.0, 0.0),
        }
    }

    fn rows_area(&self) -> (f32, f32) {
        let (header, footer) = self.header_and_footer();
        (
            VERTICAL_PADDING + header,
            self.size.height - VERTICAL_PADDING - footer,
        )
    }

    fn scrolls_up(&self) -> bool {
        self.scrolled > 0.0
    }

    fn scrolls_down(&self) -> bool {
        self.scrolled < self.scroll_limit()
    }

    fn visible_rows(&self) -> (f32, f32) {
        let (top, bottom) = self.rows_area();
        let zone = |shown: bool| if shown { SCROLL_ZONE } else { 0.0 };
        (
            top + zone(self.scrolls_up()),
            bottom - zone(self.scrolls_down()),
        )
    }

    fn scroll_zone_at(&self, point: Point) -> Option<f32> {
        if point.x < 0.0 || point.x >= self.size.width || point.y < 0.0 {
            return None;
        }
        let (top, bottom) = self.visible_rows();
        if self.scrolls_up() && point.y < top {
            return Some(-1.0);
        }
        (self.scrolls_down() && point.y >= bottom && point.y < self.size.height).then_some(1.0)
    }

    pub fn scroll_by(&mut self, delta: f32) -> bool {
        let scrolled = (self.scrolled + delta).clamp(0.0, self.scroll_limit());
        let moved = scrolled != self.scrolled;
        self.scrolled = scrolled;
        moved
    }

    fn reveal(&mut self, index: usize) {
        let Some(row) = self.current_rows().get(index) else {
            return;
        };
        let (row_top, row_bottom) = (row.top, row.top + row.height);
        let (top, bottom) = self.rows_area();
        if row_top - self.scrolled < top + SCROLL_ZONE {
            self.scrolled = row_top - top - SCROLL_ZONE;
        } else if row_bottom - self.scrolled > bottom - SCROLL_ZONE {
            self.scrolled = row_bottom - bottom + SCROLL_ZONE;
        }
        self.scrolled = self.scrolled.clamp(0.0, self.scroll_limit());
    }

    pub fn open_submenus_left(&mut self, left: bool) {
        self.submenus_open_left = left;
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
                Rect::new(0.0, row.top - self.scrolled, self.size.width, row.height)
            })
    }

    pub fn item_at(&self, point: Point) -> Option<usize> {
        let (top, bottom) = self.visible_rows();
        if point.x < 0.0 || point.x >= self.size.width || point.y < top || point.y >= bottom {
            return None;
        }
        let y = point.y + self.scrolled;
        self.current_rows()
            .iter()
            .position(|row| y >= row.top && y < row.top + row.height)
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
            MENU_RADIUS,
        );
        let mut items = vec![
            Item::Fill {
                shape: outline,
                paint: palette.overlay.into(),
            },
            Item::Stroke {
                shape: Shape::rounded(outline.rect.inset(0.25), MENU_RADIUS - 0.25),
                width: 0.5,
                paint: palette.hairline.into(),
            },
        ];
        match &self.search {
            Some(SearchLook::Query(query)) => self.paint_query(query, palette, &mut items),
            Some(SearchLook::Hint(hint, shortcut)) => {
                self.paint_hint(hint, shortcut, outline, palette, &mut items);
            }
            None => {}
        }
        let rows = self.paint_rows(palette, show_highlight);
        if !self.is_scrollable() {
            items.extend(rows);
            return items;
        }
        let (top, bottom) = self.visible_rows();
        items.push(Item::Clip {
            shape: Shape::rect(Rect::new(0.0, top, self.size.width, bottom - top)),
            items: vec![Item::Translate {
                offset: Point::new(0.0, -self.scrolled),
                items: rows,
            }],
        });
        let (area_top, area_bottom) = self.rows_area();
        let centre = self.size.width / 2.0;
        let chevron = |middle: f32, toward: f32| Item::StrokePath {
            path: Path::through(&[
                Point::new(centre - CHEVRON_HALF_WIDTH, middle - toward),
                Point::new(centre, middle + toward),
                Point::new(centre + CHEVRON_HALF_WIDTH, middle - toward),
            ]),
            width: MARK_STROKE,
            paint: palette.ink2.into(),
        };
        if self.scrolls_up() {
            items.push(chevron(area_top + SCROLL_ZONE / 2.0, -CHEVRON_RISE));
        }
        if self.scrolls_down() {
            items.push(chevron(area_bottom - SCROLL_ZONE / 2.0, CHEVRON_RISE));
        }
        items
    }

    fn paint_rows(&self, palette: &Palette, show_highlight: bool) -> Vec<Item> {
        let mut items = Vec::new();
        let highlight_radius = Radius::concentric(MENU_RADIUS, HIGHLIGHT_INSET);
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
                        SEPARATOR_INSET,
                        centre.round() - 0.5,
                        self.size.width - 2.0 * SEPARATOR_INSET,
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
                        highlight_radius,
                    ),
                    paint: palette.accent.into(),
                });
            }
            let variant = usize::from(lit);
            let mark = if lit {
                palette.on_accent
            } else if item.enabled {
                palette.ink
            } else {
                palette.ink3
            };
            if let Some(labels) = &row.labels {
                let label = &labels[variant];
                items.push(Item::Text {
                    origin: Point::new(TEXT_START, (centre - label.height() / 2.0).round()),
                    paragraph: label.clone(),
                });
            }
            let trailing = self.size.width - TRAILING_PADDING;
            if let Some(shortcuts) = &row.shortcuts {
                let shortcut = &shortcuts[variant];
                shortcut.paint(
                    Point::new(
                        (trailing - shortcut.width()).round(),
                        (centre - shortcut.height() / 2.0).round(),
                    ),
                    if lit { palette.on_accent } else { palette.ink3 },
                    &mut items,
                );
            }
            let at = |x: f32, y: f32| Point::new(x, centre + y);
            if item.checked {
                items.push(Item::FillPath {
                    path: square_capped(
                        &[at(10.0, 0.5), at(13.0, 3.5), at(18.0, -3.5)],
                        MARK_STROKE,
                    ),
                    paint: mark.into(),
                });
            }
            if item.submenu.is_some() {
                items.push(Item::StrokePath {
                    path: Path::through(&[
                        at(trailing - 3.5, -4.0),
                        at(trailing + 0.5, 0.0),
                        at(trailing - 3.5, 4.0),
                    ]),
                    width: MARK_STROKE,
                    paint: mark.into(),
                });
            }
        }
        items
    }

    fn paint_query(&self, query: &Paragraph, palette: &Palette, items: &mut Vec<Item>) {
        let centre = VERTICAL_PADDING + ROW_HEIGHT / 2.0;
        magnifier(Point::new(MAGNIFIER_CENTRE, centre), palette.ink2, items);
        let y = (centre - query.height() / 2.0).round();
        items.push(Item::Text {
            origin: Point::new(TEXT_START, y),
            paragraph: query.clone(),
        });
        let caret = (TEXT_START + query.width() + 1.0).round() + 0.75;
        items.push(Item::Line {
            from: Point::new(caret, y + 1.0),
            to: Point::new(caret, y + query.height() - 1.0),
            width: 1.5,
            paint: palette.accent.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                0.0,
                (VERTICAL_PADDING + ROW_HEIGHT + SEPARATOR_HEIGHT / 2.0).round() - 0.5,
                self.size.width,
                1.0,
            )),
            paint: palette.hairline.into(),
        });
    }

    fn paint_hint(
        &self,
        hint: &Paragraph,
        shortcut: &ShortcutText,
        outline: Shape,
        palette: &Palette,
        items: &mut Vec<Item>,
    ) {
        let tray = Rect::new(
            0.0,
            self.size.height - FOOTER_HEIGHT,
            self.size.width,
            FOOTER_HEIGHT,
        );
        let centre = tray.y + tray.height / 2.0;
        let mut footer = vec![
            Item::Fill {
                shape: Shape::rect(tray),
                paint: palette.chassis.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, tray.y, tray.width, 1.0)),
                paint: palette.hairline.into(),
            },
        ];
        magnifier(
            Point::new(MAGNIFIER_CENTRE, centre),
            palette.ink3,
            &mut footer,
        );
        footer.push(Item::Text {
            origin: Point::new(TEXT_START, (centre - hint.height() / 2.0).round()),
            paragraph: hint.clone(),
        });
        shortcut.paint(
            Point::new(
                (tray.width - TRAILING_PADDING - shortcut.width()).round(),
                (centre - shortcut.height() / 2.0).round(),
            ),
            palette.ink3,
            &mut footer,
        );
        items.push(Item::Clip {
            shape: outline,
            items: footer,
        });
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
        let mut nodes = Vec::new();
        if let Some(search) = &self.menu().search {
            if search.query.is_empty() {
                menu.set_description(search.hint.clone());
            } else {
                let mut field = Node::new(Role::SearchInput);
                field.set_label(search.hint.clone());
                field.set_value(search.query.clone());
                menu.push_child(SEARCH_NODE);
                nodes.push((SEARCH_NODE, field));
            }
        }
        nodes.insert(0, (menu_node(level, None), menu));
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

fn magnifier(centre: Point, color: Color, items: &mut Vec<Item>) {
    let lens = Point::new(centre.x - 1.0, centre.y - 1.0);
    items.push(Item::Stroke {
        shape: Shape::rounded(
            Rect::new(
                lens.x - MAGNIFIER_RADIUS,
                lens.y - MAGNIFIER_RADIUS,
                2.0 * MAGNIFIER_RADIUS,
                2.0 * MAGNIFIER_RADIUS,
            ),
            MAGNIFIER_RADIUS,
        ),
        width: MARK_STROKE,
        paint: color.into(),
    });
    let reach = MAGNIFIER_RADIUS * std::f32::consts::FRAC_1_SQRT_2;
    items.push(Item::StrokePath {
        path: Path::through(&[
            Point::new(lens.x + reach, lens.y + reach),
            Point::new(centre.x + 4.5, centre.y + 4.5),
        ]),
        width: MARK_STROKE,
        paint: color.into(),
    });
}

const MENU_NODES: u64 = u64::MAX - 1024;
const SEARCH_NODE: NodeId = NodeId(MENU_NODES + 1);

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
    autoscroll: Option<(usize, f32, Instant)>,
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
            autoscroll: None,
        }
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn searched(&mut self, panel: MenuPanel) {
        let width = self.panels[0].size().width;
        let mut panel = panel.at_least_as_wide_as(width);
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
        self.autoscroll = self
            .autoscroll
            .filter(|(level, _, _)| *level < self.panels.len());
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
        if let Some(index) = index {
            panel.reveal(index);
        }
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
        if let Some(direction) = panel.scroll_zone_at(point) {
            if self
                .autoscroll
                .is_none_or(|(known, toward, _)| known != level || toward != direction)
            {
                self.autoscroll = Some((level, direction, now));
            }
            self.pointer = Some((level, point));
            self.settle = None;
            return self.highlight(level, None, now);
        }
        if self.autoscroll.is_some_and(|(known, _, _)| known == level) {
            self.autoscroll = None;
        }
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
        let (edge, behind, moving_away) = match panel.submenus_open_left {
            true => (-SAFE_MARGIN, from.x + 1.0, point.x < from.x),
            false => (
                panel.size().width + SAFE_MARGIN,
                from.x - 1.0,
                point.x > from.x,
            ),
        };
        moving_away
            && inside_triangle(
                point,
                [
                    Point::new(behind, from.y),
                    Point::new(edge, row.y - SAFE_MARGIN),
                    Point::new(edge, row.y + height + SAFE_MARGIN),
                ],
            )
    }

    pub fn placed(&mut self, level: usize, height: f32, beside_left: bool) {
        if level > 0
            && let Some(parent) = self.panels.get_mut(level - 1)
        {
            parent.open_submenus_left(beside_left);
        }
        if let Some(panel) = self.panels.get_mut(level) {
            panel.fit_height(height);
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn scroll(&mut self, level: usize, delta: f32) -> MenuOutcome {
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        if !panel.scroll_by(delta) {
            return MenuOutcome::Nothing;
        }
        self.after_scroll(level)
    }

    fn after_scroll(&mut self, level: usize) -> MenuOutcome {
        let pointer = self
            .pointer
            .filter(|(known, _)| *known == level)
            .map(|(_, point)| point);
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        if let Some(point) = pointer {
            panel.highlighted = panel.item_at(point);
            self.submenu_due = None;
        }
        if self.panels.len() > level + 1 {
            return MenuOutcome::CloseFrom(level + 1);
        }
        MenuOutcome::Redraw
    }

    fn step_autoscroll(&mut self, now: Instant) -> MenuOutcome {
        let Some((level, direction, at)) = self.autoscroll else {
            return MenuOutcome::Nothing;
        };
        if now < at + SCROLL_STEP {
            return MenuOutcome::Nothing;
        }
        let distance = direction * SCROLL_SPEED * now.duration_since(at).as_secs_f32();
        let Some(panel) = self.panels.get_mut(level) else {
            return MenuOutcome::Nothing;
        };
        let moved = panel.scroll_by(distance);
        let more = match direction > 0.0 {
            true => panel.scrolls_down(),
            false => panel.scrolls_up(),
        };
        self.autoscroll = (moved && more).then_some((level, direction, now));
        match moved {
            true => self.after_scroll(level),
            false => MenuOutcome::Nothing,
        }
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
        let flash = self.flash.map(|(_, at)| at + 2 * FLASH_STEP + FADE_LIMIT);
        let settle = self.settle.map(|(_, _, at)| at);
        let autoscroll = self.autoscroll.map(|(_, _, at)| at + SCROLL_STEP);
        submenu
            .into_iter()
            .chain(flash)
            .chain(settle)
            .chain(autoscroll)
            .min()
    }

    pub fn tick(&mut self, now: Instant) -> MenuOutcome {
        let scrolled = self.step_autoscroll(now);
        if scrolled != MenuOutcome::Nothing {
            return scrolled;
        }
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
        self.flash.is_some_and(|(_, at)| {
            now >= at + 2 * FLASH_STEP + FADE_LIMIT || fade_after(at).value_at(now) == (0.0, false)
        })
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
        (!blink_off, fade_after(at).value_at(now).0.clamp(0.0, 1.0))
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
