use accesskit::{Action, Node, NodeId, Role, Toggled};
use bande_graphics::{
    Color, Elevation, Item, Palette, Paragraph, Path, Point, Radius, Rect, Scene, Shape, TextAlign,
    TextStyle, TextSystem,
};

use super::model::Menu;
use super::{
    MENU_RADIUS, MENU_SHADOW_MARGIN, SEARCH_NODE, TEXT_START, VERTICAL_PADDING, menu_node,
    row_height,
};
use crate::Size;
use crate::control::square_capped;
use crate::shortcut::ShortcutText;

const SEPARATOR_HEIGHT: f32 = 9.0;
const HIGHLIGHT_INSET: f32 = 4.0;
const SEPARATOR_INSET: f32 = 12.0;
const TRAILING_PADDING: f32 = 12.0;
const SHORTCUT_GAP: f32 = 28.0;
const CHEVRON_SPACE: f32 = 18.0;
const MINIMUM_WIDTH: f32 = 140.0;
const FONT_SIZE: f32 = 13.0;
const MARK_STROKE: f32 = 1.6;
const FOOTER_FONT_SIZE: f32 = 12.0;
const SEARCH_WIDTH: f32 = 220.0;
const MAGNIFIER_RADIUS: f32 = 4.0;
const MAGNIFIER_CENTRE: f32 = 14.0;
const SCROLL_ZONE: f32 = 20.0;
const FEWEST_ROWS: f32 = 3.0;
const CHEVRON_HALF_WIDTH: f32 = 4.0;
const CHEVRON_RISE: f32 = 2.0;

pub(super) struct Row {
    top: f32,
    height: f32,
    labels: Option<[Paragraph; 2]>,
    shortcuts: Option<[ShortcutText; 2]>,
}

pub struct MenuPanel {
    menus: [Menu; 2],
    rows: [Vec<Row>; 2],
    pub(super) alternates: bool,
    size: Size,
    row_height: f32,
    content_height: f32,
    scrolled: f32,
    pub(super) submenus_open_left: bool,
    pub(super) highlighted: Option<usize>,
    search: Option<SearchLook>,
}

pub(super) enum SearchLook {
    Hint(Paragraph, ShortcutText),
    Query(Paragraph),
}

impl MenuPanel {
    pub fn new(menu: Menu, text: &mut TextSystem, palette: &Palette) -> Self {
        let on_accent = palette.on_accent;
        let row_height = row_height(text.size());
        let scale = text.scale();
        let widen = |width: f32| (width * scale).round();
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
                row_height,
                (TEXT_START + hint.width() + SHORTCUT_GAP + shortcut.width() + TRAILING_PADDING)
                    .max(widen(SEARCH_WIDTH)),
            ),
            Some(SearchLook::Query(_)) => (row_height + SEPARATOR_HEIGHT, 0.0, widen(SEARCH_WIDTH)),
            None => (0.0, 0.0, widen(MINIMUM_WIDTH)),
        };
        let rows = [&menu, &alternate].map(|menu| {
            let mut top = VERTICAL_PADDING + header;
            menu.items
                .iter()
                .map(|item| {
                    let height = if item.separator {
                        SEPARATOR_HEIGHT
                    } else {
                        row_height
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
            row_height,
            content_height: height,
            scrolled: 0.0,
            submenus_open_left: false,
            highlighted: None,
            search,
        }
    }

    pub(super) fn at_least_as_wide_as(self, width: f32) -> Self {
        Self {
            size: Size::new(self.size.width.max(width), self.size.height),
            ..self
        }
    }

    pub(super) fn fit_height(&mut self, height: f32) {
        let (header, footer) = self.header_and_footer();
        let fewest = 2.0 * VERTICAL_PADDING + header + footer + 2.0 * SCROLL_ZONE;
        let fewest = fewest + FEWEST_ROWS * self.row_height;
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
            Some(SearchLook::Query(_)) => (self.row_height + SEPARATOR_HEIGHT, 0.0),
            Some(SearchLook::Hint(..)) => (0.0, self.row_height),
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

    pub(super) fn scrolls_up(&self) -> bool {
        self.scrolled > 0.0
    }

    pub(super) fn scrolls_down(&self) -> bool {
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

    pub(super) fn scroll_zone_at(&self, point: Point) -> Option<f32> {
        if point.x < 0.0 || point.x >= self.size.width || point.y < 0.0 {
            return None;
        }
        let (top, bottom) = self.visible_rows();
        if self.scrolls_up() && point.y < top {
            return Some(-1.0);
        }
        (self.scrolls_down() && point.y >= bottom && point.y < self.size.height).then_some(1.0)
    }

    pub(super) fn scroll_by(&mut self, delta: f32) -> bool {
        let scrolled = (self.scrolled + delta).clamp(0.0, self.scroll_limit());
        let moved = scrolled != self.scrolled;
        self.scrolled = scrolled;
        moved
    }

    pub(super) fn reveal(&mut self, index: usize) {
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

    pub(super) fn open_submenus_left(&mut self, left: bool) {
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

    pub(super) fn step(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let count = self.menu().items.len();
        let selectable = |index: &usize| self.menu().items[*index].selectable();
        match (from, forward) {
            (None, true) => (0..count).find(selectable),
            (None, false) => (0..count).rev().find(selectable),
            (Some(index), true) => (index + 1..count).find(selectable).or(Some(index)),
            (Some(index), false) => (0..index).rev().find(selectable).or(Some(index)),
        }
    }

    pub fn scene(&self, palette: &Palette, show_highlight: bool) -> Scene {
        let size = self.size;
        let shape = Shape::rounded(Rect::new(0.0, 0.0, size.width, size.height), MENU_RADIUS);
        let mut items = Item::shadows(shape, Elevation::Overlay, palette.scheme, true).to_vec();
        items.extend(self.paint(palette, show_highlight));
        Scene {
            width: (size.width + 2.0 * MENU_SHADOW_MARGIN) as u32,
            height: (size.height + 2.0 * MENU_SHADOW_MARGIN) as u32,
            background: Color::TRANSPARENT,
            items: vec![Item::Translate {
                offset: Point::new(MENU_SHADOW_MARGIN, MENU_SHADOW_MARGIN),
                items,
            }],
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
        let centre = VERTICAL_PADDING + self.row_height / 2.0;
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
            paint: palette.accent_text.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(
                0.0,
                (VERTICAL_PADDING + self.row_height + SEPARATOR_HEIGHT / 2.0).round() - 0.5,
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
            self.size.height - self.row_height,
            self.size.width,
            self.row_height,
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

    pub(super) fn access(&self, level: usize) -> Vec<(NodeId, Node)> {
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

pub(super) fn magnifier(centre: Point, color: Color, items: &mut Vec<Item>) {
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
