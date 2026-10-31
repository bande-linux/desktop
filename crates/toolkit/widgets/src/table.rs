use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, Node, Role, SortDirection};
use unbound_graphics::{
    Color, Image, Item, Paragraph, Point, Radius, Rect, Shape, TextAlign, TextRole, TextStyle,
    TextSystem,
};

use super::disclosure::chevron;
use super::{
    Animated, Context, Cursor, Event, Key, Menu, Modifiers, Parts, Pointer, PointerButton,
    ScrollState, Size, Spring, Widget, WidgetId,
};

pub const ROW_HEIGHT: f32 = 32.0;
pub const HEADER_HEIGHT: f32 = 28.0;
const CELL_PADDING: f32 = 8.0;
const SELECTION_INSET: (f32, f32) = (4.0, 1.0);
const ICON_SIZE: f32 = 16.0;
const ICON_GAP: f32 = 6.0;
const TEXT_SIZE: f32 = 13.0;
const RESIZE_GRIP: f32 = 4.0;
const MIN_COLUMN: f32 = 40.0;
const TYPE_SELECT_PAUSE: Duration = Duration::from_secs(1);
const ELLIPSIS: char = '…';
const DRAG_THRESHOLD: f32 = 4.0;
const INDENT: f32 = 16.0;
const DISCLOSURE_WIDTH: f32 = 16.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub title: String,
    pub width: f32,
    pub numeric: bool,
    pub sortable: bool,
    pub flexible: bool,
}

impl Column {
    pub fn new(title: &str, width: f32) -> Self {
        Self {
            title: title.to_owned(),
            width,
            numeric: false,
            sortable: true,
            flexible: false,
        }
    }

    pub fn numeric(self) -> Self {
        Self {
            numeric: true,
            ..self
        }
    }

    pub fn unsortable(self) -> Self {
        Self {
            sortable: false,
            ..self
        }
    }

    pub fn flexible(self) -> Self {
        Self {
            flexible: true,
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub text: Rc<str>,
    pub icon: Option<Image>,
    pub muted: bool,
    pub tint: Option<Color>,
    pub label: bool,
    pub tabular: bool,
}

impl Cell {
    pub fn new(text: impl Into<Rc<str>>) -> Self {
        Self {
            text: text.into(),
            icon: None,
            muted: false,
            tint: None,
            label: false,
            tabular: false,
        }
    }

    pub fn icon(self, icon: Option<Image>) -> Self {
        Self { icon, ..self }
    }

    pub fn muted(self) -> Self {
        Self {
            muted: true,
            ..self
        }
    }

    pub fn tinted(self, tint: Color) -> Self {
        Self {
            tint: Some(tint),
            ..self
        }
    }

    pub fn label(self) -> Self {
        Self {
            label: true,
            ..self
        }
    }

    pub fn tabular(self) -> Self {
        Self {
            tabular: true,
            ..self
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub key: String,
    pub cells: Vec<Cell>,
    pub children: Vec<Row>,
    pub expandable: bool,
}

impl Row {
    pub fn new(key: impl Into<String>, cells: Vec<Cell>) -> Self {
        Self {
            key: key.into(),
            cells,
            children: Vec::new(),
            expandable: false,
        }
    }

    pub fn children(self, children: Vec<Row>) -> Self {
        Self { children, ..self }
    }

    pub fn expandable(self) -> Self {
        Self {
            expandable: true,
            ..self
        }
    }
}

#[derive(Debug)]
struct Line {
    key: String,
    cells: Vec<Cell>,
    depth: usize,
    descendants: usize,
    branch: bool,
}

fn flatten(rows: Vec<Row>, depth: usize, lines: &mut Vec<Line>) -> usize {
    let mut added = 0;
    for row in rows {
        let at = lines.len();
        let branch = row.expandable || !row.children.is_empty();
        lines.push(Line {
            key: row.key,
            cells: row.cells,
            depth,
            descendants: 0,
            branch,
        });
        let descendants = flatten(row.children, depth + 1, lines);
        lines[at].descendants = descendants;
        added += 1 + descendants;
    }
    added
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort {
    pub column: usize,
    pub ascending: bool,
}

struct Remembered {
    selected: Vec<String>,
    cursor: Option<String>,
    anchor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    Press {
        from: Point,
        row: usize,
        collapse: bool,
    },
    Select,
    Resize {
        column: usize,
        from: f32,
        width: f32,
        inverted: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TextKey {
    text: Rc<str>,
    column: usize,
    color: Color,
    width: u32,
    label: bool,
    tabular: bool,
}

type KeysCallback = Box<dyn FnMut(&[String])>;
type ExpandCallback = Box<dyn FnMut(&[String], bool, bool)>;
type MenuSource = Box<dyn FnMut(&[String]) -> Option<Menu>>;
type MenuChoice = Box<dyn FnMut(&str, &[String])>;
type DragStart = Box<dyn FnMut(&[String], Point)>;

pub struct Table {
    id: WidgetId,
    columns: Vec<Column>,
    header: bool,
    lines: Vec<Line>,
    shown: Vec<usize>,
    expanded: HashSet<String>,
    turns: HashMap<String, Animated>,
    outline: bool,
    named_icons: bool,
    selected: Vec<bool>,
    cursor: Option<usize>,
    anchor: Option<usize>,
    sort: Option<Sort>,
    scroll: ScrollState,
    frame: Rect,
    drag: Option<Drag>,
    typed: String,
    typed_at: Option<Instant>,
    texts: RefCell<HashMap<TextKey, Paragraph>>,
    titles: Vec<Paragraph>,
    on_sort: Box<dyn FnMut(Sort)>,
    on_selection: KeysCallback,
    on_activate: KeysCallback,
    on_expand: ExpandCallback,
    menu: MenuSource,
    on_menu_action: MenuChoice,
    menu_keys: Vec<String>,
    on_drag: Option<DragStart>,
    offset: f32,
    pending_reveal: Option<usize>,
    glide: Option<Glide>,
}

struct Glide {
    run: (usize, usize),
    top: Animated,
    bottom: Animated,
}

impl Table {
    pub fn new(columns: Vec<Column>) -> Self {
        Self {
            id: WidgetId::next(),
            columns,
            header: true,
            lines: Vec::new(),
            shown: Vec::new(),
            expanded: HashSet::new(),
            turns: HashMap::new(),
            outline: false,
            named_icons: false,
            selected: Vec::new(),
            cursor: None,
            anchor: None,
            sort: None,
            scroll: ScrollState::default(),
            frame: Rect::default(),
            drag: None,
            typed: String::new(),
            typed_at: None,
            texts: RefCell::new(HashMap::new()),
            titles: Vec::new(),
            on_sort: Box::new(|_| {}),
            on_selection: Box::new(|_| {}),
            on_activate: Box::new(|_| {}),
            on_expand: Box::new(|_, _, _| {}),
            menu: Box::new(|_| None),
            on_menu_action: Box::new(|_, _| {}),
            menu_keys: Vec::new(),
            on_drag: None,
            offset: 0.0,
            pending_reveal: None,
            glide: None,
        }
    }

    pub fn without_header(self) -> Self {
        Self {
            header: false,
            ..self
        }
    }

    pub fn on_sort(self, callback: impl FnMut(Sort) + 'static) -> Self {
        Self {
            on_sort: Box::new(callback),
            ..self
        }
    }

    pub fn on_selection(self, callback: impl FnMut(&[String]) + 'static) -> Self {
        Self {
            on_selection: Box::new(callback),
            ..self
        }
    }

    pub fn on_activate(self, callback: impl FnMut(&[String]) + 'static) -> Self {
        Self {
            on_activate: Box::new(callback),
            ..self
        }
    }

    pub fn on_expand(self, callback: impl FnMut(&[String], bool, bool) + 'static) -> Self {
        Self {
            on_expand: Box::new(callback),
            ..self
        }
    }

    pub fn set_expanded_rows(&mut self, keys: impl IntoIterator<Item = String>) {
        let expanded = keys.into_iter().collect::<HashSet<_>>();
        if expanded != self.expanded {
            let kept = self.remembered();
            self.expanded = expanded;
            self.reshow(kept);
        }
    }

    pub fn on_drag(self, callback: impl FnMut(&[String], Point) + 'static) -> Self {
        Self {
            on_drag: Some(Box::new(callback)),
            ..self
        }
    }

    pub fn context_menu(
        self,
        menu: impl FnMut(&[String]) -> Option<Menu> + 'static,
        on_action: impl FnMut(&str, &[String]) + 'static,
    ) -> Self {
        Self {
            menu: Box::new(menu),
            on_menu_action: Box::new(on_action),
            ..self
        }
    }

    pub fn set_sort(&mut self, sort: Option<Sort>) {
        self.sort = sort;
    }

    pub fn set_columns(&mut self, columns: Vec<Column>) {
        self.columns = columns;
        self.sort = None;
        self.drag = None;
        self.texts.borrow_mut().clear();
    }

    pub fn set_rows(&mut self, rows: Vec<Row>) {
        let kept = self.remembered();
        let mut lines = Vec::with_capacity(rows.len());
        flatten(rows, 0, &mut lines);
        self.outline = lines.iter().any(|line| line.branch);
        self.named_icons = lines
            .iter()
            .any(|line| line.cells.first().is_some_and(|cell| cell.icon.is_some()));
        self.lines = lines;
        self.reshow(kept);
    }

    fn is_flat(&self) -> bool {
        self.shown.len() == self.lines.len()
    }

    pub fn insert_row(&mut self, index: usize, row: Row) {
        let mut lines = Vec::new();
        flatten(vec![row], 0, &mut lines);
        let index = index.min(self.lines.len());
        let added = lines.len();
        let kept = (!self.is_flat() || added > 1).then(|| self.remembered());
        self.outline |= lines.iter().any(|line| line.branch);
        self.named_icons |= lines
            .iter()
            .any(|line| line.cells.first().is_some_and(|cell| cell.icon.is_some()));
        self.lines.splice(index..index, lines);
        match kept {
            Some(kept) => self.reshow(kept),
            None => {
                self.shown.push(self.shown.len());
                self.selected.insert(index, false);
                let shift = |at: usize| if at >= index { at + 1 } else { at };
                self.cursor = self.cursor.map(shift);
                self.anchor = self.anchor.map(shift);
            }
        }
    }

    pub fn remove_row(&mut self, index: usize) {
        let Some(line) = self.lines.get(index) else {
            return;
        };
        let removed = 1 + line.descendants;
        if !self.is_flat() || removed > 1 {
            let kept = self.remembered();
            self.lines.drain(index..index + removed);
            self.reshow(kept);
            return;
        }
        self.lines.remove(index);
        self.shown.truncate(self.lines.len());
        self.selected.remove(index);
        let shift = |at: usize| match at.cmp(&index) {
            std::cmp::Ordering::Less => Some(at),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(at - 1),
        };
        self.cursor = self.cursor.and_then(shift);
        self.anchor = self.anchor.and_then(shift);
    }

    pub fn select_rows(&mut self, rows: &[usize]) {
        self.selected = vec![false; self.shown.len()];
        for row in rows {
            if let Some(selected) = self.selected.get_mut(*row) {
                *selected = true;
            }
        }
        self.cursor = rows.first().copied().filter(|row| *row < self.shown.len());
        self.anchor = self.cursor;
    }

    pub fn row_count(&self) -> usize {
        self.shown.len()
    }

    pub fn selected_rows(&self) -> Vec<usize> {
        self.selected
            .iter()
            .enumerate()
            .filter(|(_, selected)| **selected)
            .map(|(index, _)| index)
            .collect()
    }

    fn key_at(&self, index: usize) -> Option<&str> {
        self.line(index).map(|line| line.key.as_str())
    }

    pub fn selection(&self) -> Vec<String> {
        self.shown
            .iter()
            .zip(&self.selected)
            .filter(|(_, selected)| **selected)
            .map(|(line, _)| self.lines[*line].key.clone())
            .collect()
    }

    pub fn select(&mut self, keys: &[String]) {
        let keys = keys.iter().map(String::as_str).collect::<HashSet<_>>();
        self.selected = self
            .shown
            .iter()
            .map(|line| keys.contains(self.lines[*line].key.as_str()))
            .collect();
        self.cursor = self.selected.iter().position(|selected| *selected);
        self.anchor = self.cursor;
    }

    fn line(&self, index: usize) -> Option<&Line> {
        self.shown.get(index).map(|line| &self.lines[*line])
    }

    fn remembered(&self) -> Remembered {
        let key = |index: Option<usize>| {
            index
                .and_then(|index| self.line(index))
                .map(|line| line.key.clone())
        };
        Remembered {
            selected: self.selection(),
            cursor: key(self.cursor),
            anchor: key(self.anchor),
        }
    }

    fn reshow(&mut self, kept: Remembered) {
        let mut shown = Vec::with_capacity(self.lines.len());
        let mut index = 0;
        while let Some(line) = self.lines.get(index) {
            shown.push(index);
            index += match line.branch && !self.expanded.contains(&line.key) {
                true => 1 + line.descendants,
                false => 1,
            };
        }
        self.shown = shown;
        if kept.selected.is_empty() && kept.cursor.is_none() && kept.anchor.is_none() {
            self.cursor = None;
            self.anchor = None;
            self.selected = vec![false; self.shown.len()];
            return;
        }
        let positions = self
            .shown
            .iter()
            .enumerate()
            .map(|(index, line)| (self.lines[*line].key.as_str(), index))
            .collect::<HashMap<_, _>>();
        let position =
            |key: &Option<String>| key.as_deref().and_then(|key| positions.get(key).copied());
        let cursor = position(&kept.cursor);
        let anchor = position(&kept.anchor);
        let mut selected = vec![false; self.shown.len()];
        for key in &kept.selected {
            if let Some(index) = positions.get(key.as_str()) {
                selected[*index] = true;
            }
        }
        self.cursor = cursor;
        self.anchor = anchor;
        self.selected = selected;
    }

    fn ancestor(&self, index: usize) -> Option<usize> {
        let depth = self.line(index)?.depth;
        (0..index)
            .rev()
            .find(|candidate| self.line(*candidate).is_some_and(|line| line.depth < depth))
    }

    fn set_expanded(&mut self, cx: &Context, index: usize, expanded: bool, all: bool) -> bool {
        let Some(&at) = self.shown.get(index) else {
            return false;
        };
        let line = &self.lines[at];
        if !line.branch {
            return false;
        }
        let subtree = match all {
            true => at..at + 1 + line.descendants,
            false => at..at + 1,
        };
        let mut changed = Vec::new();
        for line in &self.lines[subtree] {
            if line.branch && self.expanded.contains(&line.key) != expanded {
                changed.push(line.key.clone());
            }
        }
        if changed.is_empty() {
            return false;
        }
        for key in &changed {
            let mut turn = Animated::new(if expanded { 0.0 } else { 1.0 });
            turn.animate_to(cx, if expanded { 1.0 } else { 0.0 }, Spring::QUICK);
            self.turns.insert(key.clone(), turn);
        }
        self.change_expansion(at, &changed, expanded);
        (self.on_expand)(&changed, expanded, all);
        true
    }

    pub fn is_row_expanded(&self, key: &str) -> bool {
        self.expanded.contains(key)
    }

    pub fn set_row_expanded(&mut self, key: &str, expanded: bool) -> bool {
        let Some(at) = self
            .lines
            .iter()
            .position(|line| line.branch && line.key == key)
        else {
            return false;
        };
        if self.expanded.contains(key) == expanded {
            return false;
        }
        self.turns.remove(key);
        self.change_expansion(at, &[key.to_owned()], expanded);
        true
    }

    fn change_expansion(&mut self, at: usize, changed: &[String], expanded: bool) {
        let mut kept = self.remembered();
        let before = kept.selected.len();
        for key in changed {
            match expanded {
                true => self.expanded.insert(key.clone()),
                false => self.expanded.remove(key),
            };
        }
        let key = self.lines[at].key.clone();
        let mut moved = false;
        if !expanded {
            let hidden = self.lines[at + 1..at + 1 + self.lines[at].descendants]
                .iter()
                .map(|line| line.key.as_str())
                .collect::<HashSet<_>>();
            kept.selected
                .retain(|selected| !hidden.contains(selected.as_str()));
            moved = kept.selected.len() != before;
            if moved && !kept.selected.contains(&key) {
                kept.selected.push(key.clone());
            }
            if kept
                .cursor
                .as_deref()
                .is_some_and(|cursor| hidden.contains(cursor))
            {
                kept.cursor = Some(key.clone());
                kept.anchor = Some(key);
            }
        }
        self.reshow(kept);
        if moved {
            self.notify_selection();
        }
    }

    fn turn(&self, cx: &Context, line: &Line) -> f32 {
        match self.turns.get(&line.key) {
            Some(turn) => turn.value(cx),
            None => match self.expanded.contains(&line.key) {
                true => 1.0,
                false => 0.0,
            },
        }
    }

    fn indent(&self, line: &Line) -> f32 {
        match self.outline {
            true => line.depth as f32 * INDENT + DISCLOSURE_WIDTH,
            false => 0.0,
        }
    }

    fn disclosure_at(&self, index: usize, point: Point) -> bool {
        let Some(line) = self.line(index).filter(|line| line.branch) else {
            return false;
        };
        let x = self.frame.x + CELL_PADDING + line.depth as f32 * INDENT;
        point.x >= x - 2.0 && point.x < x + DISCLOSURE_WIDTH
    }

    fn header_height(&self) -> f32 {
        match self.header {
            true => HEADER_HEIGHT,
            false => 0.0,
        }
    }

    fn body(&self) -> Rect {
        Rect::new(
            self.frame.x,
            self.frame.y + self.header_height(),
            self.frame.width,
            (self.frame.height - self.header_height()).max(0.0),
        )
    }

    fn stretched(&self) -> usize {
        self.columns
            .iter()
            .position(|column| column.flexible)
            .unwrap_or(self.columns.len().saturating_sub(1))
    }

    fn widths(&self) -> Vec<f32> {
        let stretched = self.stretched();
        let fixed: f32 = self
            .columns
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != stretched)
            .map(|(_, column)| column.width)
            .sum();
        self.columns
            .iter()
            .enumerate()
            .map(|(index, column)| match index == stretched {
                true => (self.frame.width - fixed).max(column.width.min(MIN_COLUMN * 2.0)),
                false => column.width,
            })
            .collect()
    }

    fn column_edges(&self) -> Vec<(f32, f32)> {
        let mut x = self.frame.x;
        self.widths()
            .into_iter()
            .map(|width| {
                let edge = (x, width);
                x += width;
                edge
            })
            .collect()
    }

    pub fn row_at(&self, point: Point) -> Option<usize> {
        self.row_at_offset(point, self.offset)
    }

    pub fn row_bounds(&self, index: usize) -> Option<Rect> {
        let body = self.body();
        (index < self.shown.len()).then(|| {
            Rect::new(
                body.x,
                body.y + index as f32 * ROW_HEIGHT - self.offset,
                body.width,
                ROW_HEIGHT,
            )
        })
    }

    pub fn cell_bounds(&self, index: usize, column: usize) -> Option<Rect> {
        let row = self.row_bounds(index)?;
        let (x, width) = *self.column_edges().get(column)?;
        Some(Rect::new(x, row.y, width, row.height))
    }

    pub fn cursor_row(&self) -> Option<usize> {
        self.cursor
    }

    pub fn scroll_offset(&self) -> f32 {
        self.offset
    }

    pub fn scroll_to_row(&mut self, index: usize) {
        self.pending_reveal = Some(index);
    }

    fn row_at_offset(&self, point: Point, offset: f32) -> Option<usize> {
        let body = self.body();
        if !body.contains(point) {
            return None;
        }
        let y = point.y - body.y + offset;
        let index = (y / ROW_HEIGHT).floor();
        (index >= 0.0 && (index as usize) < self.shown.len()).then_some(index as usize)
    }

    fn row_under(&self, cx: &Context, point: Point) -> Option<usize> {
        self.row_at_offset(point, self.scroll.offset(cx))
    }

    fn notify_selection(&mut self) {
        let keys = self.selection();
        (self.on_selection)(&keys);
    }

    fn reveal_row(&mut self, cx: &Context, index: usize) {
        let top = index as f32 * ROW_HEIGHT;
        self.scroll.reveal(cx, top, top + ROW_HEIGHT);
    }

    fn select_only(&mut self, index: usize) {
        self.selected = vec![false; self.shown.len()];
        if let Some(selected) = self.selected.get_mut(index) {
            *selected = true;
        }
        self.cursor = Some(index);
        self.anchor = Some(index);
    }

    fn select_range(&mut self, to: usize) {
        let anchor = *self.anchor.get_or_insert(to);
        let (low, high) = (anchor.min(to), anchor.max(to));
        self.selected = (0..self.shown.len())
            .map(|index| (low..=high).contains(&index))
            .collect();
        self.cursor = Some(to);
    }

    fn click(&mut self, index: Option<usize>, modifiers: Modifiers, count: u8) -> bool {
        let Some(index) = index else {
            let had = self.selected.iter().any(|selected| *selected);
            self.selected = vec![false; self.shown.len()];
            self.cursor = None;
            self.anchor = None;
            if had {
                self.notify_selection();
            }
            return had;
        };
        if modifiers.shift {
            self.select_range(index);
        } else if modifiers.logo {
            if let Some(selected) = self.selected.get_mut(index) {
                *selected = !*selected;
            }
            self.cursor = Some(index);
            self.anchor = Some(index);
        } else if !(count >= 2 && self.selected.get(index).copied().unwrap_or(false)) {
            self.select_only(index);
        }
        self.notify_selection();
        if count >= 2 && !modifiers.shift && !modifiers.logo {
            let keys = self.selection();
            (self.on_activate)(&keys);
        }
        true
    }

    fn step(&mut self, cx: &Context, to: usize, extend: bool) -> bool {
        let to = to.min(self.shown.len().saturating_sub(1));
        if self.shown.is_empty() {
            return false;
        }
        if extend {
            self.select_range(to);
        } else {
            self.select_only(to);
        }
        self.reveal_row(cx, to);
        self.notify_selection();
        true
    }

    fn page_rows(&self) -> usize {
        ((self.body().height / ROW_HEIGHT).floor() as usize).max(1)
    }

    fn key(&mut self, cx: &mut Context, key: Key, modifiers: Modifiers) -> bool {
        let last = self.shown.len().saturating_sub(1);
        let cursor = self.cursor;
        let extend = modifiers.shift;
        match key {
            Key::Up if modifiers.logo => self.step(cx, 0, extend),
            Key::Down if modifiers.logo => self.step(cx, last, extend),
            Key::Up => self.step(
                cx,
                cursor.map_or(last, |index| index.saturating_sub(1)),
                extend,
            ),
            Key::Down => self.step(cx, cursor.map_or(0, |index| index + 1), extend),
            Key::Home => self.step(cx, 0, extend),
            Key::End => self.step(cx, last, extend),
            Key::PageUp => {
                let page = self.page_rows();
                self.step(cx, cursor.unwrap_or(0).saturating_sub(page), extend)
            }
            Key::PageDown => {
                let page = self.page_rows();
                self.step(cx, cursor.unwrap_or(0) + page, extend)
            }
            Key::Right if self.outline => {
                cursor.is_some_and(|index| self.right(cx, index, modifiers.alt))
            }
            Key::Left if self.outline => {
                cursor.is_some_and(|index| self.left(cx, index, modifiers.alt))
            }
            Key::Enter if !self.selection().is_empty() => {
                let keys = self.selection();
                (self.on_activate)(&keys);
                true
            }
            Key::Character('c') if modifiers.logo && !self.selection().is_empty() => {
                let text = self
                    .shown
                    .iter()
                    .zip(&self.selected)
                    .filter(|(_, selected)| **selected)
                    .map(|(line, _)| {
                        self.lines[*line]
                            .cells
                            .iter()
                            .map(|cell| cell.text.as_ref())
                            .collect::<Vec<_>>()
                            .join("\t")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                cx.copy(&text);
                true
            }
            Key::Character('a') if modifiers.logo => {
                self.selected = vec![true; self.shown.len()];
                self.notify_selection();
                true
            }
            Key::Character(character)
                if !modifiers.logo && !modifiers.control && !character.is_control() =>
            {
                self.type_select(cx, character)
            }
            _ => false,
        }
    }

    fn right(&mut self, cx: &Context, index: usize, all: bool) -> bool {
        let Some(line) = self.line(index).filter(|line| line.branch) else {
            return false;
        };
        match self.expanded.contains(&line.key) && !all {
            true => self.step(cx, index + 1, false),
            false => self.set_expanded(cx, index, true, all),
        }
    }

    fn left(&mut self, cx: &Context, index: usize, all: bool) -> bool {
        let Some(line) = self.line(index) else {
            return false;
        };
        if line.branch && (self.expanded.contains(&line.key) || all) {
            return self.set_expanded(cx, index, false, all);
        }
        match self.ancestor(index) {
            Some(parent) => self.step(cx, parent, false),
            None => false,
        }
    }

    fn type_select(&mut self, cx: &Context, character: char) -> bool {
        let now = cx.now();
        if self
            .typed_at
            .is_none_or(|at| now.saturating_duration_since(at) > TYPE_SELECT_PAUSE)
        {
            self.typed.clear();
        }
        self.typed_at = Some(now);
        self.typed.extend(character.to_lowercase());
        let typed = self.typed.clone();
        let found = self.shown.iter().position(|line| {
            self.lines[*line]
                .cells
                .first()
                .is_some_and(|cell| cell.text.to_lowercase().starts_with(&typed))
        });
        match found {
            Some(index) => self.step(cx, index, false),
            None => false,
        }
    }

    fn header_click(&mut self, point: Point) -> bool {
        let Some(column) = self
            .column_edges()
            .iter()
            .position(|(x, width)| point.x >= *x && point.x < x + width)
        else {
            return false;
        };
        if !self.columns[column].sortable {
            return false;
        }
        let ascending = match self.sort {
            Some(sort) if sort.column == column => !sort.ascending,
            _ => !self.columns[column].numeric,
        };
        let sort = Sort { column, ascending };
        self.sort = Some(sort);
        (self.on_sort)(sort);
        true
    }

    fn resize_grip(&self, point: Point) -> Option<usize> {
        if point.y < self.frame.y || point.y >= self.frame.y + self.header_height() {
            return None;
        }
        let edges = self.column_edges();
        edges
            .iter()
            .take(edges.len().saturating_sub(1))
            .position(|(x, width)| (point.x - (x + width)).abs() <= RESIZE_GRIP)
    }

    fn pointer_down(&mut self, cx: &mut Context, pointer: &Pointer) -> bool {
        if pointer.button == PointerButton::Primary {
            if let Some(grip) = self.resize_grip(pointer.position) {
                let inverted = grip >= self.stretched();
                let column = grip + usize::from(inverted);
                self.drag = Some(Drag::Resize {
                    column,
                    from: pointer.position.x,
                    width: self.columns[column].width,
                    inverted,
                });
                return true;
            }
            if pointer.position.y < self.frame.y + self.header_height() {
                return self.header_click(pointer.position);
            }
        }
        let index = self.row_under(cx, pointer.position);
        if let Some(row) = index.filter(|row| {
            pointer.is_primary() && self.outline && self.disclosure_at(*row, pointer.position)
        }) {
            let expanded = self
                .key_at(row)
                .is_some_and(|key| self.expanded.contains(key));
            return self.set_expanded(cx, row, !expanded, pointer.modifiers.alt);
        }
        if pointer.button == PointerButton::Secondary {
            let on_selection =
                index.is_some_and(|index| self.selected.get(index).copied().unwrap_or(false));
            if !on_selection {
                self.click(index, Modifiers::default(), 1);
            }
            return true;
        }
        let modifiers = pointer.modifiers;
        let plain = !modifiers.shift && !modifiers.logo;
        let collapse = plain
            && pointer.count == 1
            && index.is_some_and(|index| self.selected.get(index).copied().unwrap_or(false));
        self.drag = Some(match index {
            Some(row) => Drag::Press {
                from: pointer.position,
                row,
                collapse,
            },
            None => Drag::Select,
        });
        if collapse {
            self.cursor = index;
            return true;
        }
        self.click(index, modifiers, pointer.count)
    }

    fn pointer_up(&mut self) -> bool {
        match self.drag.take() {
            Some(Drag::Press {
                row,
                collapse: true,
                ..
            }) => {
                self.select_only(row);
                self.notify_selection();
                true
            }
            _ => false,
        }
    }

    fn press_moved(&mut self, cx: &mut Context, from: Point, row: usize, point: Point) -> bool {
        let distance = (point.x - from.x).hypot(point.y - from.y);
        if distance < DRAG_THRESHOLD {
            return false;
        }
        if self.on_drag.is_some() {
            self.drag = None;
            let keys = self.selection();
            if let Some(on_drag) = &mut self.on_drag {
                on_drag(&keys, from);
            }
            return true;
        }
        if self.row_under(cx, point) == Some(row) {
            return false;
        }
        if let Some(Drag::Press { collapse: true, .. }) = self.drag {
            self.select_only(row);
        }
        self.drag = Some(Drag::Select);
        self.pointer_move(cx, point)
    }

    fn pointer_move(&mut self, cx: &mut Context, point: Point) -> bool {
        match self.drag {
            Some(Drag::Resize {
                column,
                from,
                width,
                inverted,
            }) => {
                let moved = match inverted {
                    true => from - point.x,
                    false => point.x - from,
                };
                self.columns[column].width = (width + moved).max(MIN_COLUMN);
                true
            }
            Some(Drag::Press { from, row, .. }) => self.press_moved(cx, from, row, point),
            Some(Drag::Select) => {
                let body = self.body();
                let clamped =
                    Point::new(point.x, point.y.clamp(body.y, body.y + body.height - 1.0));
                if point.y < body.y || point.y > body.y + body.height {
                    let edge = if point.y < body.y {
                        -ROW_HEIGHT
                    } else {
                        ROW_HEIGHT
                    };
                    let offset = self.scroll.offset(cx);
                    self.scroll.scroll_to(cx, offset + edge);
                }
                match self.row_under(cx, clamped) {
                    Some(index) if self.cursor != Some(index) => {
                        self.select_range(index);
                        self.notify_selection();
                        true
                    }
                    _ => false,
                }
            }
            None => false,
        }
    }

    fn text(
        &self,
        text: &mut TextSystem,
        content: &str,
        style: TextStyle,
        width: f32,
    ) -> Paragraph {
        let paragraph = text.paragraph(content, style, None, TextAlign::Start);
        if paragraph.width() <= width {
            return paragraph;
        }
        let characters = content.chars().collect::<Vec<_>>();
        let (mut low, mut high) = (0, characters.len());
        while low < high {
            let middle = (low + high).div_ceil(2);
            let candidate = characters[..middle].iter().collect::<String>() + &ELLIPSIS.to_string();
            if text
                .paragraph(&candidate, style, None, TextAlign::Start)
                .width()
                <= width
            {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        let shortened = characters[..low]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_owned()
            + &ELLIPSIS.to_string();
        text.paragraph(&shortened, style, None, TextAlign::Start)
    }

    fn text_key(
        &self,
        cx: &Context,
        line: &Line,
        column: usize,
        lit: bool,
        widths: &[f32],
    ) -> TextKey {
        let cell = &line.cells[column];
        let color = match (lit, cell.tint, cell.muted) {
            (true, _, _) => cx.palette.on_accent,
            (false, Some(tint), _) => tint,
            (false, None, true) => cx.palette.ink2,
            (false, None, false) => cx.palette.ink,
        };
        let indent = if column == 0 { self.indent(line) } else { 0.0 };
        let width = widths[column] - 2.0 * CELL_PADDING - self.icon_space(column, cell) - indent;
        let width = width.max(0.0);
        TextKey {
            text: cell.text.clone(),
            column,
            color,
            width: width.to_bits(),
            label: cell.label,
            tabular: cell.tabular || self.columns[column].numeric,
        }
    }

    fn icon_space(&self, column: usize, cell: &Cell) -> f32 {
        match cell.icon.is_some() || (column == 0 && self.named_icons) {
            true => ICON_SIZE + ICON_GAP,
            false => 0.0,
        }
    }

    fn prepare_texts(&self, cx: &mut Context, first: usize, last: usize) {
        let focused = cx.is_focused(self.id);
        let widths = self.widths();
        let mut old = std::mem::take(&mut *self.texts.borrow_mut());
        let mut texts = HashMap::with_capacity(old.len());
        for index in first..last {
            let lit = focused && self.lit(cx, index);
            let Some(line) = self.line(index) else {
                continue;
            };
            for column in 0..line.cells.len().min(self.columns.len()) {
                let key = self.text_key(cx, line, column, lit, &widths);
                if texts.contains_key(&key) {
                    continue;
                }
                let paragraph = match old.remove(&key) {
                    Some(paragraph) => paragraph,
                    None => {
                        let style = match key.label {
                            true => TextStyle::role(TextRole::Label, key.color),
                            false => TextStyle::new(TEXT_SIZE, key.color),
                        };
                        let style = match key.tabular {
                            true => style.tabular(),
                            false => style,
                        };
                        self.text(&mut cx.text, &key.text, style, f32::from_bits(key.width))
                    }
                };
                texts.insert(key, paragraph);
            }
        }
        *self.texts.borrow_mut() = texts;
    }

    fn visible_rows(&self, cx: &Context) -> (usize, usize) {
        let offset = self.scroll.offset(cx).max(0.0);
        let first = (offset / ROW_HEIGHT).floor() as usize;
        let count = (self.body().height / ROW_HEIGHT).ceil() as usize + 2;
        (
            first.min(self.shown.len()),
            (first + count).min(self.shown.len()),
        )
    }

    fn paint_header(&self, cx: &Context, items: &mut Vec<Item>) {
        let header = Rect::new(self.frame.x, self.frame.y, self.frame.width, HEADER_HEIGHT);
        items.push(Item::Fill {
            shape: Shape::rect(header),
            paint: cx.palette.sheet.into(),
        });
        for (column, ((x, width), paragraph)) in self
            .column_edges()
            .into_iter()
            .zip(&self.titles)
            .enumerate()
        {
            let sorted = self.sort.filter(|sort| sort.column == column);
            let paragraph = paragraph.clone();
            let text_x = match self.columns[column].numeric {
                true => x + width - CELL_PADDING - paragraph.width(),
                false => x + CELL_PADDING,
            };
            let text_y = self.frame.y + ((HEADER_HEIGHT - paragraph.height()) / 2.0).round();
            if let Some(sort) = sorted {
                let arrow_x = match self.columns[column].numeric {
                    true => text_x - 10.0,
                    false => text_x + paragraph.width() + 6.0,
                };
                let middle = self.frame.y + HEADER_HEIGHT / 2.0;
                let (tip, base) = if sort.ascending {
                    (-2.0, 2.0)
                } else {
                    (2.0, -2.0)
                };
                for side in [-1.0, 1.0] {
                    items.push(Item::Line {
                        from: Point::new(arrow_x + 2.5, middle + tip),
                        to: Point::new(arrow_x + 2.5 + side * 3.0, middle + base),
                        width: 1.5,
                        paint: cx.palette.ink2.into(),
                    });
                }
            }
            items.push(Item::Text {
                origin: Point::new(text_x, text_y),
                paragraph,
            });
        }
        items.push(Item::Line {
            from: Point::new(self.frame.x, self.frame.y + HEADER_HEIGHT - 0.5),
            to: Point::new(
                self.frame.x + self.frame.width,
                self.frame.y + HEADER_HEIGHT - 0.5,
            ),
            width: 1.0,
            paint: cx.palette.hairline.into(),
        });
    }

    fn runs(&self) -> Vec<(usize, usize)> {
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (index, selected) in self.selected.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if *selected && run.1 == index => run.1 = index + 1,
                _ if *selected => runs.push((index, index + 1)),
                _ => {}
            }
        }
        runs
    }

    fn lit(&self, cx: &Context, index: usize) -> bool {
        match &self.glide {
            Some(glide) => {
                let centre = (index as f32 + 0.5) * ROW_HEIGHT;
                glide.top.value(cx) <= centre && centre < glide.bottom.value(cx)
            }
            None => self.selected.get(index).copied().unwrap_or(false),
        }
    }

    fn follow_selection(&mut self, cx: &Context) {
        let runs = self.runs();
        let [run] = runs[..] else {
            self.glide = None;
            return;
        };
        let (top, bottom) = (run.0 as f32 * ROW_HEIGHT, run.1 as f32 * ROW_HEIGHT);
        match &mut self.glide {
            Some(glide) if glide.run == run => {}
            Some(glide) => {
                glide.run = run;
                glide.top.animate_to(cx, top, Spring::QUICK);
                glide.bottom.animate_to(cx, bottom, Spring::QUICK);
            }
            None => {
                self.glide = Some(Glide {
                    run,
                    top: Animated::points(top),
                    bottom: Animated::points(bottom),
                });
            }
        }
    }

    fn paint_selection(
        &self,
        cx: &Context,
        body: Rect,
        offset: f32,
        focused: bool,
        items: &mut Vec<Item>,
    ) {
        let color = match focused {
            true => cx.palette.accent,
            false => cx.palette.well,
        };
        let spans: Vec<(f32, f32)> = match &self.glide {
            Some(glide) => vec![(glide.top.value(cx), glide.bottom.value(cx))],
            None => self
                .runs()
                .into_iter()
                .map(|(first, last)| (first as f32 * ROW_HEIGHT, last as f32 * ROW_HEIGHT))
                .collect(),
        };
        for (top, bottom) in spans {
            let rect = Rect::new(
                body.x + SELECTION_INSET.0,
                body.y + top - offset + SELECTION_INSET.1,
                body.width - 2.0 * SELECTION_INSET.0,
                bottom - top - 2.0 * SELECTION_INSET.1,
            );
            items.push(Item::Fill {
                shape: Shape::rounded(rect, Radius::S),
                paint: color.into(),
            });
        }
    }

    fn header_parts(&self, parts: &mut Parts) {
        let header_id = self.id.part(u64::from(u32::MAX - 1));
        let mut header = Node::new(Role::Row);
        let mut titles = Vec::new();
        for (column, (x, width)) in self.column_edges().into_iter().enumerate() {
            let id = self.id.part(u64::from(u32::MAX - 2) - column as u64);
            let mut title = Node::new(Role::ColumnHeader);
            title.set_label(self.columns[column].title.clone());
            title.set_bounds(access_rect(Rect::new(
                x,
                self.frame.y,
                width,
                HEADER_HEIGHT,
            )));
            if let Some(sort) = self.sort.filter(|sort| sort.column == column) {
                title.set_sort_direction(match sort.ascending {
                    true => SortDirection::Ascending,
                    false => SortDirection::Descending,
                });
            }
            if self.columns[column].sortable {
                title.add_action(Action::Click);
            }
            titles.push(id);
            parts.nodes.push((id, title));
        }
        header.set_children(titles);
        parts.children.push(header_id);
        parts.nodes.push((header_id, header));
    }

    fn row_node(&self, index: usize, row: &Line, parts: &mut Parts, bounds: Rect) {
        let row_id = self.id.part(index as u64 * (self.columns.len() as u64 + 1));
        let mut node = Node::new(Role::Row);
        node.set_row_index(index);
        if self.outline {
            node.set_level(row.depth + 1);
        }
        if row.branch {
            let expanded = self.expanded.contains(&row.key);
            node.set_expanded(expanded);
            node.add_action(match expanded {
                true => Action::Collapse,
                false => Action::Expand,
            });
        }
        node.set_selected(self.selected.get(index).copied().unwrap_or(false));
        node.add_action(Action::Click);
        node.add_action(Action::Focus);
        node.add_action(Action::ScrollIntoView);
        node.set_bounds(access_rect(bounds));
        let mut cells = Vec::new();
        for (column, cell) in row.cells.iter().enumerate().take(self.columns.len()) {
            let cell_id = self
                .id
                .part(index as u64 * (self.columns.len() as u64 + 1) + column as u64 + 1);
            let mut cell_node = Node::new(Role::Cell);
            cell_node.set_label(cell.text.as_ref());
            cell_node.set_column_index(column);
            cell_node.set_row_index(index);
            cells.push(cell_id);
            parts.nodes.push((cell_id, cell_node));
        }
        if let Some(name) = row.cells.first() {
            node.set_label(name.text.as_ref());
        }
        node.set_children(cells);
        parts.children.push(row_id);
        parts.nodes.push((row_id, node));
    }
}

fn access_rect(rect: Rect) -> accesskit::Rect {
    accesskit::Rect {
        x0: f64::from(rect.x),
        y0: f64::from(rect.y),
        x1: f64::from(rect.x + rect.width),
        y1: f64::from(rect.y + rect.height),
    }
}

impl Widget for Table {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let body = self.body();
        self.scroll
            .set_extent(self.shown.len() as f32 * ROW_HEIGHT, body.height);
        if let Some(index) = self.pending_reveal.take() {
            self.reveal_row(cx, index);
        }
        self.offset = self.scroll.offset(cx);
        self.follow_selection(cx);
        self.turns
            .retain(|_, turn| (turn.value(cx) - turn.target()).abs() > f32::EPSILON);
        let (first, last) = self.visible_rows(cx);
        self.prepare_texts(cx, first, last);
        self.titles = self
            .column_edges()
            .into_iter()
            .enumerate()
            .map(|(column, (_, width))| {
                let sorted = self.sort.is_some_and(|sort| sort.column == column);
                let color = match sorted {
                    true => cx.palette.ink2,
                    false => cx.palette.ink3,
                };
                let style = TextStyle::role(TextRole::Label, color);
                let title = self.columns[column].title.to_uppercase();
                self.text(
                    &mut cx.text,
                    &title,
                    style,
                    (width - 2.0 * CELL_PADDING - if sorted { 12.0 } else { 0.0 }).max(0.0),
                )
            })
            .collect();
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let body = self.body();
        let offset = self.scroll.offset(cx);
        let focused = cx.is_focused(self.id);
        let (first, last) = self.visible_rows(cx);
        let widths = self.widths();
        let edges = self.column_edges();
        let texts = self.texts.borrow();
        let mut rows = Vec::new();
        self.paint_selection(cx, body, offset, focused, &mut rows);
        for index in first..last {
            let y = body.y + index as f32 * ROW_HEIGHT - offset;
            let lit = focused && self.lit(cx, index);
            let Some(line) = self.line(index) else {
                continue;
            };
            if line.branch {
                let centre = Point::new(
                    self.frame.x
                        + CELL_PADDING
                        + line.depth as f32 * INDENT
                        + DISCLOSURE_WIDTH / 2.0
                        - 2.0,
                    y + ROW_HEIGHT / 2.0,
                );
                let color = match lit {
                    true => cx.palette.on_accent,
                    false => cx.palette.ink2,
                };
                chevron(centre, self.turn(cx, line), color, &mut rows);
            }
            for (column, cell) in line.cells.iter().enumerate().take(self.columns.len()) {
                let (x, width) = edges[column];
                let mut text_x = x + CELL_PADDING;
                if column == 0 {
                    text_x += self.indent(line);
                }
                if let Some(icon) = &cell.icon {
                    rows.push(Item::Image {
                        rect: Rect::new(
                            text_x,
                            y + (ROW_HEIGHT - ICON_SIZE) / 2.0,
                            ICON_SIZE,
                            ICON_SIZE,
                        ),
                        image: icon.clone(),
                    });
                }
                text_x += self.icon_space(column, cell);
                let key = self.text_key(cx, line, column, lit, &widths);
                let Some(paragraph) = texts.get(&key) else {
                    continue;
                };
                if self.columns[column].numeric {
                    text_x = x + widths[column].min(width) - CELL_PADDING - paragraph.width();
                }
                rows.push(Item::Text {
                    origin: Point::new(
                        text_x,
                        y + ((ROW_HEIGHT - paragraph.height()) / 2.0).round(),
                    ),
                    paragraph: paragraph.clone(),
                });
            }
        }
        items.push(Item::Clip {
            shape: Shape::rect(body),
            items: rows,
        });
        if self.header {
            let mut header = Vec::new();
            self.paint_header(cx, &mut header);
            items.push(Item::Clip {
                shape: Shape::rect(Rect::new(
                    self.frame.x,
                    self.frame.y,
                    self.frame.width,
                    HEADER_HEIGHT,
                )),
                items: header,
            });
        }
        self.scroll.paint_scrollbar(cx, body, items);
    }

    fn access(&self, cx: &Context) -> Node {
        let mut node = Node::new(match self.outline {
            true => Role::TreeGrid,
            false => Role::Table,
        });
        node.set_row_count(self.shown.len());
        node.set_column_count(self.columns.len());
        node.set_multiselectable();
        self.scroll.describe(cx, &mut node);
        node
    }

    fn parts(&self, cx: &Context) -> Parts {
        let mut parts = Parts::default();
        if self.header {
            self.header_parts(&mut parts);
        }
        let (first, last) = self.visible_rows(cx);
        let offset = self.scroll.offset(cx);
        let body = self.body();
        let mut shown = (first..last).collect::<Vec<_>>();
        if let Some(cursor) = self.cursor.filter(|cursor| !shown.contains(cursor)) {
            shown.push(cursor);
        }
        for index in shown {
            let Some(row) = self.line(index) else {
                continue;
            };
            let bounds = Rect::new(
                body.x,
                body.y + index as f32 * ROW_HEIGHT - offset,
                body.width,
                ROW_HEIGHT,
            );
            self.row_node(index, row, &mut parts, bounds);
        }
        if cx.is_focused(self.id) {
            parts.focus = self
                .cursor
                .map(|index| self.id.part(index as u64 * (self.columns.len() as u64 + 1)));
        }
        parts
    }

    fn perform_part(&mut self, cx: &mut Context, part: u64, request: &ActionRequest) -> bool {
        let stride = self.columns.len() as u64 + 1;
        let header_start = u64::from(u32::MAX - 2) - self.columns.len() as u64;
        if part > header_start && part <= u64::from(u32::MAX - 2) {
            let column = (u64::from(u32::MAX - 2) - part) as usize;
            let x = self.column_edges().get(column).map_or(0.0, |(x, _)| *x);
            return request.action == Action::Click
                && self.header_click(Point::new(x + 1.0, self.frame.y + 1.0));
        }
        let index = (part / stride) as usize;
        if index >= self.shown.len() {
            return false;
        }
        match request.action {
            Action::Click | Action::Focus => self.step(cx, index, false),
            Action::Expand => self.set_expanded(cx, index, true, false),
            Action::Collapse => self.set_expanded(cx, index, false, false),
            Action::ScrollIntoView => {
                self.reveal_row(cx, index);
                true
            }
            _ => false,
        }
    }

    fn handle(&mut self, cx: &mut Context, event: &Event) -> bool {
        match event {
            Event::Scroll(scroll) => self.scroll.scroll(cx, scroll),
            Event::PointerDown(pointer) => {
                if self.scroll.pointer(cx, self.body(), event) {
                    return true;
                }
                self.pointer_down(cx, pointer)
            }
            Event::PointerMove(pointer) => {
                self.scroll.pointer(cx, self.body(), event)
                    | self.pointer_move(cx, pointer.position)
            }
            Event::PointerUp(_) => self.pointer_up() | self.scroll.pointer(cx, self.body(), event),
            Event::PointerLeave => {
                self.drag = None;
                false
            }
            Event::Key { key, modifiers } => self.key(cx, *key, *modifiers),
            _ => false,
        }
    }

    fn perform(&mut self, cx: &mut Context, request: &ActionRequest) -> bool {
        self.scroll.perform(cx, request)
    }

    fn accepts_focus(&self, _cx: &Context) -> bool {
        true
    }

    fn cursor(&self, point: Point) -> Option<Cursor> {
        self.resize_grip(point).map(|_| Cursor::ColumnResize)
    }

    fn context_menu(&mut self, _cx: &mut Context, _point: Point) -> Option<Menu> {
        self.menu_keys = self.selection();
        (self.menu)(&self.menu_keys)
    }

    fn menu_action(&mut self, _cx: &mut Context, action: &str) -> bool {
        let keys = std::mem::take(&mut self.menu_keys);
        (self.on_menu_action)(action, &keys);
        true
    }
}
