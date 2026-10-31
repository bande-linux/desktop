use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Instant, SystemTime};

use accesskit::{Action, ActionRequest, TreeId};
use unbound_fs::{Entry, Sort, SortBy, format_date, format_list_date, format_size};
use unbound_toolkit::ui::{
    Alert, Cell, Column, Event, FreeLayout, HEADER_HEIGHT, Key, Menu, MenuItem, Modifiers, Pointer,
    PointerButton, Row, SLIDER_HEIGHT, Size, Slider, Sort as TableSort, Table, TextField, Toggle,
    Widget, WidgetId,
};
use unbound_toolkit::{Image, Point, Rect};

use crate::L10N;
use crate::browser::{Location, RowEdit};
use crate::prompt::{Kind, Prompt};
use crate::view::{
    COLUMN_WIDTH, FilesView, PATH_BAR_HEIGHT, ROW_HEIGHT, Rename, Request, SLOW_CLICK_RENAME,
    Sheet, contains,
};
use crate::views::{ICON_SIZES, ListColumn, ViewMode, save_list_columns};

pub(crate) const COLUMN_ACTION: &str = "column:";

#[derive(Default)]
pub(crate) struct Formats {
    types: HashMap<String, (Option<Image>, Rc<str>)>,
    sizes: HashMap<u64, Rc<str>>,
    dates: HashMap<u64, Rc<str>>,
    dash: Option<Rc<str>>,
}

const NAME_SIZE: f32 = 13.0;
const ROW_INSET: f32 = 6.0;
const NAME_INSET: f32 = 28.0;
const TABLE_TEXT_INSET: f32 = 6.0 + 16.0 + 6.0;
const FIELD_INSET: f32 = 3.0;
pub(crate) const SLIDER_WIDTH: f32 = 96.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Signal {
    Choice(String),
    Selection,
    Activate,
    Sort(TableSort),
    Search(String),
    GoTo(String),
    ApplyToAll(bool),
    Drag(Point),
    IconSize(f64),
    View(usize),
    Scope(bool),
    Back,
    Forward,
    Expand(Vec<String>, bool, bool),
}

pub(crate) type Signals = Rc<RefCell<Vec<Signal>>>;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Press {
    Ui,
    Table {
        rename: Option<PathBuf>,
    },
    Files,
    Item {
        at: Point,
        collapse: Option<PathBuf>,
    },
    Dragging,
    Band,
    Place {
        index: usize,
        at: Point,
    },
}

pub(crate) fn sender(signals: &Signals) -> impl Fn(Signal) + 'static {
    let signals = signals.clone();
    move |signal| signals.borrow_mut().push(signal)
}

pub(crate) fn table(signals: &Signals) -> Table {
    let (selection, activate, sort, drag, expand) = (
        sender(signals),
        sender(signals),
        sender(signals),
        sender(signals),
        sender(signals),
    );
    Table::new(table_columns(false, &ListColumn::DEFAULT))
        .on_selection(move |_| selection(Signal::Selection))
        .on_activate(move |_| activate(Signal::Activate))
        .on_sort(move |order| sort(Signal::Sort(order)))
        .on_drag(move |_, at| drag(Signal::Drag(at)))
        .on_expand(move |keys, expanded, all| expand(Signal::Expand(keys.to_vec(), expanded, all)))
}

fn nested_rows(rows: Vec<Row>, depths: &[u8]) -> Vec<Row> {
    fn close(open: &mut Vec<Row>, roots: &mut Vec<Row>) {
        if let Some(done) = open.pop() {
            match open.last_mut() {
                Some(parent) => parent.children.push(done),
                None => roots.push(done),
            }
        }
    }
    let mut roots = Vec::new();
    let mut open: Vec<Row> = Vec::new();
    for (row, depth) in rows.into_iter().zip(depths) {
        while open.len() > usize::from(*depth) {
            close(&mut open, &mut roots);
        }
        open.push(row);
    }
    while !open.is_empty() {
        close(&mut open, &mut roots);
    }
    roots
}

fn column_title(column: ListColumn) -> String {
    L10N.tr(match column {
        ListColumn::Modified => "column-modified",
        ListColumn::Created => "column-created",
        ListColumn::Size => "column-size",
        ListColumn::Kind => "column-kind",
    })
}

fn table_columns(found: bool, shown: &[ListColumn]) -> Vec<Column> {
    let mut columns = match found {
        false => vec![Column::new(&L10N.tr("column-name"), 280.0)],
        true => vec![
            Column::new(&L10N.tr("column-name"), 200.0).flexible(),
            Column::new(&L10N.tr("column-where"), 170.0).unsortable(),
        ],
    };
    columns.extend(shown.iter().map(|column| {
        let title = column_title(*column);
        match (column, found) {
            (ListColumn::Modified | ListColumn::Created, false) => Column::new(&title, 170.0),
            (ListColumn::Modified | ListColumn::Created, true) => Column::new(&title, 150.0),
            (ListColumn::Size, false) => Column::new(&title, 80.0).numeric(),
            (ListColumn::Size, true) => Column::new(&title, 70.0).numeric(),
            (ListColumn::Kind, false) => Column::new(&title, 160.0),
            (ListColumn::Kind, true) => Column::new(&title, 120.0),
        }
    }));
    columns
}

fn table_sorts(found: bool, shown: &[ListColumn]) -> Vec<Option<SortBy>> {
    let mut sorts = vec![Some(SortBy::Name)];
    if found {
        sorts.push(None);
    }
    sorts.extend(shown.iter().map(|column| Some(column.sort())));
    sorts
}

fn cached_date(time: SystemTime, formats: &mut Formats) -> Rc<str> {
    let minute = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() / 60);
    formats
        .dates
        .entry(minute)
        .or_insert_with(|| Rc::from(format_list_date(time)))
        .clone()
}

pub(crate) fn columns_menu(shown: &[ListColumn]) -> Menu {
    Menu::new(
        ListColumn::ALL
            .iter()
            .map(|column| {
                MenuItem::new(
                    &column_title(*column),
                    &format!("{COLUMN_ACTION}{}", column.name()),
                )
                .checked(shown.contains(column))
            })
            .collect(),
    )
}

fn shown_folder(folder: &Path, scope: &Path) -> String {
    let base = scope.parent().unwrap_or(scope);
    folder
        .strip_prefix(base)
        .unwrap_or(folder)
        .iter()
        .map(|part| part.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" \u{203A} ")
}

pub(crate) fn key_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn type_key(name: &str, folder: bool) -> Cow<'_, str> {
    let key = match (folder, name.split_once('.')) {
        (true, _) => "",
        (false, Some((_, suffix))) => suffix,
        (false, None) => name,
    };
    match key.chars().any(char::is_uppercase) {
        true => Cow::Owned(key.to_lowercase()),
        false => Cow::Borrowed(key),
    }
}

fn table_key(key: Key, modifiers: Modifiers) -> bool {
    match key {
        Key::Up | Key::Down | Key::Home | Key::End | Key::PageUp | Key::PageDown => !modifiers.logo,
        Key::Character('a' | 'A') if modifiers.logo => !modifiers.shift && !modifiers.alt,
        Key::Character(_) => !modifiers.logo && !modifiers.control && !modifiers.alt,
        _ => false,
    }
}

fn stem_end(name: &str, folder: bool) -> usize {
    match folder {
        true => name.len(),
        false => Path::new(name)
            .file_stem()
            .map_or(name.len(), |stem| stem.len()),
    }
}

fn request(action: Action, target: accesskit::NodeId) -> ActionRequest {
    ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    }
}

impl FilesView {
    fn host_mut(&mut self) -> Option<&mut FreeLayout> {
        self.ui.widget_as::<FreeLayout>(self.host)
    }

    pub(crate) fn frame_of(&mut self, id: WidgetId) -> Option<Rect> {
        self.host_mut().and_then(|host| host.frame_of(id))
    }

    pub(crate) fn focus(&mut self, id: WidgetId) {
        self.ui.perform(&request(Action::Focus, id.into()));
    }

    pub(crate) fn focus_content(&mut self) {
        match self.mode == ViewMode::List {
            true => self.focus(self.table),
            false => self.ui.clear_focus(),
        }
    }

    pub(crate) fn insert(&mut self, widget: impl Widget + 'static) -> WidgetId {
        let id = widget.id();
        if let Some(host) = self.host_mut() {
            host.insert(widget, Rect::default());
        }
        self.layout_controls();
        id
    }

    pub(crate) fn remove(&mut self, id: WidgetId) {
        if let Some(host) = self.host_mut() {
            host.remove(id);
        }
    }

    fn search_focused(&self) -> bool {
        self.ui.focus() == Some(self.toolbar.search)
    }

    pub(crate) fn replace_widget(
        &mut self,
        old: WidgetId,
        widget: impl Widget + 'static,
    ) -> WidgetId {
        let focused = self.ui.focus() == Some(old);
        self.remove(old);
        let id = self.insert(widget);
        if focused {
            self.focus(id);
        }
        id
    }

    fn controls(&self) -> Vec<WidgetId> {
        [
            Some(self.toolbar.back),
            Some(self.toolbar.forward),
            Some(self.toolbar.view),
            Some(self.toolbar.search),
            self.size_slider,
            self.searching
                .as_ref()
                .map(|searching| searching.scope_control),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub(crate) fn route(&mut self, event: &Event) -> bool {
        if self.ui.has_sheet() {
            return self.ui.handle(event);
        }
        match event {
            Event::Key { key, modifiers } => self.route_key(*key, *modifiers),
            Event::PointerDown(pointer) => self.pointer_down(event, pointer),
            Event::PointerMove(pointer) | Event::PointerEnter(pointer) => match self.press {
                Some(Press::Ui | Press::Table { .. }) => self.ui.handle(event),
                Some(Press::Item { at, .. }) => self.maybe_drag(at, pointer),
                Some(Press::Band) => self.band_moved(pointer),
                Some(Press::Place { index, at }) => self.reorder_moved(index, at, pointer),
                Some(_) => false,
                None => self.ui.handle(event) | self.files_hover(pointer),
            },
            Event::PointerUp(_) => self.pointer_up(event),
            Event::PointerLeave => {
                self.press = None;
                self.ui.handle(event) | self.leave()
            }
            Event::Scroll(scroll) => {
                self.ui.handle(event)
                    || (!self.over_control(scroll.position) && self.files_scroll(scroll))
            }
            _ => self.ui.handle(event),
        }
    }

    fn route_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if self.rename.is_some() {
            return self.rename_key(key, modifiers);
        }
        if self.search_focused() {
            return self.search_key(key, modifiers);
        }
        if self.mode == ViewMode::List
            && table_key(key, modifiers)
            && self.ui.handle(&Event::Key { key, modifiers })
        {
            return true;
        }
        if self.key(key, modifiers) {
            return true;
        }
        self.mode == ViewMode::List
            && key != Key::Tab
            && self.ui.handle(&Event::Key { key, modifiers })
    }

    fn over_control(&mut self, at: Point) -> bool {
        let ids = [Some(self.table)]
            .into_iter()
            .chain([self.rename.as_ref().map(|rename| rename.field)])
            .flatten()
            .chain(self.controls())
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| self.frame_of(id))
            .any(|frame| contains(frame, at))
    }

    fn pointer_down(&mut self, event: &Event, pointer: &Pointer) -> bool {
        let at = pointer.position;
        if matches!(pointer.button, PointerButton::Back | PointerButton::Forward) {
            self.press = None;
            return self.files_pointer_down(pointer);
        }
        if let Some(field) = self.rename.as_ref().map(|rename| rename.field) {
            if self
                .frame_of(field)
                .is_some_and(|frame| contains(frame, at))
            {
                self.press = Some(Press::Ui);
                return self.ui.handle(event);
            }
            self.commit_rename();
        }
        let on_control = self
            .controls()
            .into_iter()
            .filter_map(|id| self.frame_of(id))
            .any(|frame| contains(frame, at));
        if on_control {
            self.press = Some(Press::Ui);
            return self.ui.handle(event);
        }
        if let Some(table) = self
            .frame_of(self.table)
            .filter(|frame| contains(*frame, at))
        {
            return self.table_down(event, pointer, table);
        }
        self.press = Some(Press::Files);
        if self.search_focused() {
            self.focus_content();
        }
        self.files_pointer_down(pointer)
    }

    fn table_down(&mut self, event: &Event, pointer: &Pointer, table: Rect) -> bool {
        let at = pointer.position;
        let menu = pointer.button == PointerButton::Secondary
            || (pointer.is_primary() && pointer.modifiers.control);
        if at.y < table.y + HEADER_HEIGHT && menu && !self.browser.searching() {
            let menu = columns_menu(&self.list_columns);
            self.show_menu(menu, at);
            return true;
        }
        if at.y < table.y + HEADER_HEIGHT {
            self.press = Some(Press::Ui);
            return self.ui.handle(event);
        }
        self.pending_rename = None;
        let row = self.table_row_at(at);
        let column = &self.browser.columns()[0];
        let modifiers = pointer.modifiers;
        let plain = !modifiers.shift && !modifiers.logo && !modifiers.control;
        let rename = row
            .filter(|row| {
                pointer.is_primary()
                    && plain
                    && pointer.count == 1
                    && column.is_marked(*row)
                    && column.marked.len() == 1
            })
            .and_then(|row| column.entries.get(row))
            .map(|entry| entry.path.clone());
        self.press = Some(Press::Table { rename });
        let handled = self.ui.handle(event);
        self.drain_signals();
        let menu = pointer.button == PointerButton::Secondary
            || (pointer.is_primary() && modifiers.control);
        if menu {
            let menu = match row {
                Some(_) => self.selection_menu(),
                None => self.folder_menu(),
            };
            self.show_menu(menu, at);
        }
        handled
    }

    fn pointer_up(&mut self, event: &Event) -> bool {
        match self.press.take() {
            Some(Press::Table { rename }) => {
                let handled = self.ui.handle(event);
                self.drain_signals();
                if let Some(path) = rename {
                    self.pending_rename = Some((Instant::now() + SLOW_CLICK_RENAME, path));
                }
                handled
            }
            Some(Press::Item { collapse, .. }) => {
                let Some(path) = collapse else {
                    return false;
                };
                let found =
                    self.browser
                        .columns()
                        .iter()
                        .enumerate()
                        .find_map(|(index, column)| {
                            let row = column.entries.iter().position(|entry| entry.path == path)?;
                            Some((index, row))
                        });
                if let Some((column, row)) = found {
                    self.browser.select(column, row);
                    self.selection_changed();
                }
                true
            }
            Some(Press::Band) => self.end_band(),
            Some(Press::Place { .. }) => self.reorder_finished(),
            Some(Press::Files | Press::Dragging) => false,
            _ => self.ui.handle(event),
        }
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let performed = self.ui.perform(request);
        self.after_ui();
        performed
    }

    pub fn accessibility(&self) -> accesskit::TreeUpdate {
        self.ui.accessibility()
    }

    pub fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    pub(crate) fn after_ui(&mut self) {
        if let Some((menu, at)) = self.ui.take_menu_request() {
            self.requests.push(Request::ShowMenu { menu, at });
        }
        if self.ui.take_emoji_picker_request() {
            self.requests.push(Request::ShowEmojiPicker);
        }
        if let Some(request) = self.ui.take_clipboard_request() {
            self.requests.push(Request::Clipboard(request));
        }
        self.drain_signals();
    }

    pub(crate) fn drain_signals(&mut self) {
        loop {
            let signals = std::mem::take(&mut *self.signals.borrow_mut());
            if signals.is_empty() {
                return;
            }
            for signal in signals {
                match signal {
                    Signal::Choice(action) => self.choose(&action),
                    Signal::Selection => self.table_selected(),
                    Signal::Expand(keys, expanded, all) => {
                        let folders = keys.iter().map(PathBuf::from).collect::<Vec<_>>();
                        self.browser.set_expanded(&folders, expanded, all);
                        self.selection_changed();
                    }
                    Signal::Activate => self.open(),
                    Signal::Sort(order) => self.table_sorted(order),
                    Signal::Search(text) => self.search_changed(&text),
                    Signal::View(index) => self.view_chosen(index),
                    Signal::Back => self.back(),
                    Signal::Forward => self.forward(),
                    Signal::Scope(everywhere) => self.set_scope(everywhere),
                    Signal::IconSize(size) => self.set_icon_size(size.round() as u32),
                    Signal::GoTo(text) => {
                        if let Some(Sheet::GoTo(typed)) = &mut self.sheet {
                            *typed = text;
                        }
                    }
                    Signal::Drag(at) => {
                        self.ui.handle(&Event::PointerLeave);
                        self.start_drag(at);
                    }
                    Signal::ApplyToAll(on) => {
                        if let Some(Sheet::Conflict { apply_to_all }) = &mut self.sheet {
                            *apply_to_all = on;
                        }
                    }
                }
            }
        }
    }

    fn table_selected(&mut self) {
        if self.mode != ViewMode::List {
            return;
        }
        let column = &self.browser.columns()[0];
        let rows = match self.shown_table() {
            Some(table) if table.row_count() == column.entries.len() => table.selected_rows(),
            Some(table) => {
                let keys = table.selection();
                column
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| keys.contains(&key_of(&entry.path)))
                    .map(|(row, _)| row)
                    .collect()
            }
            None => Vec::new(),
        };
        self.browser.mark(0, rows);
        self.table_revealed = self.browser.active_column().selected;
        self.selection_changed();
    }

    fn table_sorted(&mut self, order: TableSort) {
        let found = self.browser.searching();
        let sorts = table_sorts(found, &self.list_columns);
        let Some(by) = sorts.get(order.column).copied().flatten() else {
            return;
        };
        self.browser.set_sort(Sort {
            by,
            descending: !order.ascending,
        });
        self.remember_view();
        self.selection_changed();
    }

    fn table(&mut self) -> Option<&mut Table> {
        self.ui.widget_as::<Table>(self.table)
    }

    pub(crate) fn shown_table(&self) -> Option<&Table> {
        self.ui.widget_ref::<Table>(self.table)
    }

    pub(crate) fn toggle_list_column(&mut self, column: ListColumn) {
        match self.list_columns.iter().position(|shown| *shown == column) {
            Some(index) => {
                self.list_columns.remove(index);
            }
            None => {
                self.list_columns.push(column);
                self.list_columns
                    .sort_by_key(|shown| ListColumn::ALL.iter().position(|known| known == shown));
            }
        }
        if let Some(file) = &self.list_columns_file
            && let Err(error) = save_list_columns(file, &self.list_columns)
        {
            tracing::warn!("cannot remember the list columns: {error}");
        }
        self.sync_table();
    }

    fn table_row(&mut self, entry: &Entry, scope: Option<&Path>, formats: &mut Formats) -> Row {
        let folder = entry.is_folder();
        let type_key = type_key(&entry.name, folder);
        let (icon, kind) = match formats.types.get(type_key.as_ref()) {
            Some(known) => known.clone(),
            None => {
                let known = (
                    self.entry_icon(entry, 32),
                    Rc::from(self.apps.kind_for_name(&entry.name, folder)),
                );
                formats.types.insert(type_key.into_owned(), known.clone());
                known
            }
        };
        let size = match folder {
            true => formats
                .dash
                .get_or_insert_with(|| Rc::from("\u{2014}"))
                .clone(),
            false => formats
                .sizes
                .entry(entry.size)
                .or_insert_with(|| Rc::from(format_size(entry.size)))
                .clone(),
        };
        let modified = entry
            .modified
            .map(|time| cached_date(time, formats))
            .unwrap_or_default();
        let mut cells = Vec::with_capacity(self.list_columns.len() + 2);
        cells.push(Cell::new(entry.name.as_str()).icon(icon));
        for column in &self.list_columns {
            let text = match column {
                ListColumn::Modified => modified.clone(),
                ListColumn::Created => entry
                    .created
                    .map(|time| cached_date(time, formats))
                    .unwrap_or_default(),
                ListColumn::Size => size.clone(),
                ListColumn::Kind => kind.clone(),
            };
            cells.push(Cell::new(text).muted());
        }
        if let Some(scope) = scope {
            let folder = entry
                .path
                .parent()
                .map(|folder| shown_folder(folder, scope))
                .unwrap_or_default();
            cells.insert(1, Cell::new(folder).muted());
        }
        let row = Row::new(key_of(&entry.path), cells);
        match folder && matches!(self.browser.root(), Location::Folder(_)) {
            true => row.expandable(),
            false => row,
        }
    }

    fn table_rows(&mut self, rows: impl IntoIterator<Item = usize>) -> Vec<(usize, Row)> {
        let scope = match self.browser.root() {
            Location::Search(scope) => Some(scope.clone()),
            _ => None,
        };
        let entries = std::mem::take(&mut self.browser.columns_mut()[0].entries);
        let mut formats = std::mem::take(&mut self.table_formats);
        let built = rows
            .into_iter()
            .filter_map(|row| {
                let entry = entries.get(row)?;
                Some((row, self.table_row(entry, scope.as_deref(), &mut formats)))
            })
            .collect();
        self.table_formats = formats;
        self.browser.columns_mut()[0].entries = entries;
        built
    }

    fn update_table_rows(&mut self) {
        let column = &self.browser.columns()[0];
        let (revision, edits) = (column.revision, column.edits.len());
        let patchable = !column.is_nested()
            && matches!(self.table_synced, Some((shown, applied)) if shown == revision && applied <= edits);
        if !patchable {
            self.table_formats = Formats::default();
            let count = self.browser.columns()[0].entries.len();
            let rows = self
                .table_rows(0..count)
                .into_iter()
                .map(|(_, row)| row)
                .collect::<Vec<_>>();
            let column = &self.browser.columns()[0];
            let rows = match column.is_nested() {
                true => nested_rows(rows, &column.depths),
                false => rows,
            };
            let expanded = column
                .entries
                .iter()
                .filter(|entry| entry.is_folder() && self.browser.is_expanded(&entry.path))
                .map(|entry| key_of(&entry.path))
                .collect::<Vec<_>>();
            if let Some(table) = self.table() {
                table.set_expanded_rows(expanded);
                table.set_rows(rows);
            }
            self.table_revealed = None;
        } else if let Some((_, applied)) = self.table_synced {
            let pending = self.browser.columns()[0].edits[applied..].to_vec();
            for edit in pending {
                match edit {
                    RowEdit::Remove(row) => {
                        if let Some(table) = self.table() {
                            table.remove_row(row);
                        }
                    }
                    RowEdit::Insert(row) => {
                        let built = self.table_rows([row]);
                        if let Some(((_, built), table)) =
                            built.into_iter().next().zip(self.table())
                        {
                            table.insert_row(row, built);
                        }
                    }
                }
            }
        }
        self.table_synced = Some((revision, edits));
    }

    pub(crate) fn sync_table(&mut self) {
        self.layout_controls();
        if self.mode != ViewMode::List {
            return;
        }
        let found = self.browser.searching();
        let shape = (found, self.list_columns.clone());
        if self.table_shape != shape {
            self.table_synced = None;
            let columns = table_columns(found, &shape.1);
            self.table_shape = shape;
            if let Some(table) = self.table() {
                table.set_columns(columns);
            }
        }
        self.update_table_rows();
        let sort = self.browser.sort();
        let column = &self.browser.columns()[0];
        let cursor = column.selected;
        let wanted = column.marked.clone();
        let shown = table_sorts(found, &self.list_columns)
            .iter()
            .position(|by| *by == Some(sort.by))
            .filter(|_| *self.browser.root() != Location::Recents)
            .map(|column| TableSort {
                column,
                ascending: !sort.descending,
            });
        if let Some(table) = self.table() {
            table.set_sort(shown);
            if table.selected_rows() != wanted {
                table.select_rows(&wanted);
            }
        }
        if cursor != self.table_revealed {
            self.table_revealed = cursor;
            if let Some((row, table)) = cursor.zip(self.table()) {
                table.scroll_to_row(row);
            }
        }
    }

    pub(crate) fn table_row_bounds(&self, row: usize) -> Option<Rect> {
        self.shown_table()?.row_bounds(row)
    }

    pub(crate) fn table_row_at(&self, at: Point) -> Option<usize> {
        self.shown_table()?.row_at(at)
    }

    fn rename_frame(&mut self, path: &Path) -> Rect {
        let content = self.content_frame();
        if self.mode == ViewMode::Icons {
            let row = self.browser.columns()[0]
                .entries
                .iter()
                .position(|entry| entry.path == path);
            return row.map_or_else(Rect::default, |row| self.grid_rename_frame(row));
        }
        if self.mode == ViewMode::List {
            let row = self.browser.columns()[0]
                .entries
                .iter()
                .position(|entry| entry.path == path);
            let Some(name) = row.and_then(|row| self.shown_table()?.cell_bounds(row, 0)) else {
                return Rect::default();
            };
            let x = name.x + TABLE_TEXT_INSET - FIELD_INSET;
            return Rect::new(
                x,
                name.y + 2.0,
                (name.x + name.width - x - 4.0).max(0.0),
                ROW_HEIGHT - 4.0,
            );
        }
        let first = self.first_visible_column();
        let found = self
            .browser
            .columns()
            .iter()
            .enumerate()
            .skip(first)
            .find_map(|(index, column)| {
                let row = column.entries.iter().position(|entry| entry.path == path)?;
                Some((index, row))
            });
        let Some((index, row)) = found else {
            return Rect::default();
        };
        let scroll = self.column_offset(index);
        let x = content.x + (index - first) as f32 * COLUMN_WIDTH + ROW_INSET;
        let y = content.y + self.rows_top() + row as f32 * ROW_HEIGHT - scroll;
        let name_x = x + NAME_INSET - FIELD_INSET;
        Rect::new(
            name_x,
            y + 2.0,
            x + COLUMN_WIDTH - 2.0 * ROW_INSET - name_x - 2.0,
            ROW_HEIGHT - 4.0,
        )
    }

    pub(crate) fn layout_controls(&mut self) {
        let content = self.content_frame();
        let table = match self.mode == ViewMode::List {
            true => content,
            false => Rect::default(),
        };
        let search = self.toolbar_frames().into_iter().chain(self.scope_frame());
        let rename = self
            .rename
            .as_ref()
            .map(|rename| (rename.field, rename.path.clone()));
        let rename = rename.map(|(field, path)| (field, self.rename_frame(&path)));
        let slider = self.size_slider.map(|slider| {
            let bar_y = self.size.1 - PATH_BAR_HEIGHT;
            (
                slider,
                Rect::new(
                    content.x + content.width - SLIDER_WIDTH - 14.0,
                    bar_y + ((PATH_BAR_HEIGHT - SLIDER_HEIGHT) / 2.0).round(),
                    SLIDER_WIDTH,
                    SLIDER_HEIGHT,
                ),
            )
        });
        let table_id = self.table;
        if let Some(host) = self.host_mut() {
            host.set_frame(table_id, table);
            for (id, frame) in search.into_iter().chain(rename).chain(slider) {
                host.set_frame(id, frame);
            }
        }
    }

    pub fn start_rename(&mut self) {
        if self.browser.in_trash() && self.browser.active() == 0 {
            return;
        }
        let Some(entry) = self.browser.selected() else {
            return;
        };
        let (path, name, folder) = (entry.path.clone(), entry.name.clone(), entry.is_folder());
        self.cancel_rename();
        let mut field = TextField::new("", &name, |_| {}).in_place(NAME_SIZE);
        field.select(0..stem_end(&name, folder));
        let field = self.insert(field);
        self.rename = Some(Rename { path, field });
        self.focus(field);
        self.ui
            .layout(Size::new(self.size.0.max(1.0), self.size.1.max(1.0)));
    }

    pub(crate) fn cancel_rename(&mut self) {
        if let Some(rename) = self.rename.take() {
            self.remove(rename.field);
            self.focus_content();
        }
    }

    pub(crate) fn commit_rename(&mut self) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        let name = self
            .ui
            .widget_as::<TextField>(rename.field)
            .map(|field| field.text().to_owned());
        self.remove(rename.field);
        self.focus_content();
        if let Some(name) = name {
            self.rename_to(&rename.path, &name);
        }
    }

    pub(crate) fn refresh_size_slider(&mut self) {
        if let Some(slider) = self.size_slider.take() {
            self.remove(slider);
        }
        if self.mode != ViewMode::Icons || !self.status_bar {
            return;
        }
        let send = sender(&self.signals);
        let slider = Slider::new(
            &L10N.tr("icon-size"),
            f64::from(self.icon_size),
            (f64::from(ICON_SIZES.0), f64::from(ICON_SIZES.1)),
            8.0,
            move |size| send(Signal::IconSize(size)),
        );
        self.size_slider = Some(self.insert(slider));
    }

    fn choice(&self) -> impl Fn(&str) + 'static {
        let send = sender(&self.signals);
        move |action: &str| send(Signal::Choice(action.to_owned()))
    }

    pub(crate) fn present(&mut self, sheet: Sheet) {
        let choose = self.choice();
        let alert = match &sheet {
            Sheet::EmptyTrash => Alert::new(
                &L10N.tr("empty-trash-title"),
                &L10N.tr("cannot-undo"),
                choose,
            )
            .cancel(&L10N.tr("cancel"), "cancel")
            .primary(&L10N.tr("empty-trash"), "confirm"),
            Sheet::DeleteImmediately { paths, items } => Alert::new(
                &L10N.tr_args(
                    "delete-title",
                    [("count", (paths.len() + items.len()).into())],
                ),
                &L10N.tr("cannot-undo"),
                choose,
            )
            .cancel(&L10N.tr("cancel"), "cancel")
            .primary(&L10N.tr("delete"), "confirm"),
            Sheet::Conflict { .. } | Sheet::GoTo(_) => return,
        };
        self.show_sheet(alert, sheet, None);
    }

    fn show_sheet(
        &mut self,
        widget: impl Widget + 'static,
        sheet: Sheet,
        cancel: Option<WidgetId>,
    ) {
        self.commit_rename();
        self.quick_look = None;
        self.ui.present_sheet(widget);
        self.ui
            .layout(Size::new(self.size.0.max(1.0), self.size.1.max(1.0)));
        self.sheet = Some(sheet);
        self.sheet_cancel = cancel;
    }

    pub(crate) fn ask_go_to(&mut self) {
        let current = self
            .browser
            .current_folder()
            .map(|folder| folder.display().to_string())
            .unwrap_or_default();
        let send = sender(&self.signals);
        let mut field = TextField::new("", &current, move |text| {
            send(Signal::GoTo(text.to_owned()));
        });
        field.select(0..current.len());
        let prompt = Prompt::new(&L10N.tr("go-to-title"), "", self.choice())
            .accessory(field)
            .button(&L10N.tr("cancel"), "cancel", Kind::Cancel)
            .button(&L10N.tr("go"), "go", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.show_sheet(prompt, Sheet::GoTo(current), cancel);
    }

    pub(crate) fn present_conflict(&mut self) {
        let Some((_, running)) = self.operations.conflict() else {
            return;
        };
        let Some(conflict) = running.conflict.clone() else {
            return;
        };
        let several = running.items > 1;
        let name = conflict
            .target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let describe = |path: &Path| {
            std::fs::symlink_metadata(path)
                .ok()
                .map(|metadata| {
                    let modified = metadata.modified().map(format_date).unwrap_or_default();
                    match metadata.is_dir() {
                        true => modified,
                        false => format!("{}, {modified}", format_size(metadata.len())),
                    }
                })
                .unwrap_or_default()
        };
        let detail = L10N.tr_args(
            "conflict-detail",
            [
                ("existing", describe(&conflict.target).into()),
                ("new", describe(&conflict.source).into()),
            ],
        );
        let mut prompt = Prompt::new(
            &L10N.tr_args("conflict-title", [("name", name.into())]),
            &detail,
            self.choice(),
        );
        if several {
            let send = sender(&self.signals);
            let label = L10N.tr("apply-to-all");
            prompt = prompt.toggle(
                &label,
                Toggle::new(&label, false, move |on| send(Signal::ApplyToAll(on))),
            );
        }
        let prompt = prompt
            .button(&L10N.tr("skip"), "skip", Kind::Other)
            .button(&L10N.tr("replace"), "replace", Kind::Other)
            .button(&L10N.tr("stop"), "stop", Kind::Cancel)
            .button(&L10N.tr("keep-both"), "keep-both", Kind::Primary);
        let cancel = prompt.cancel_button();
        self.show_sheet(
            prompt,
            Sheet::Conflict {
                apply_to_all: false,
            },
            cancel,
        );
    }

    pub(crate) fn dismiss_sheet(&mut self) {
        self.sheet = None;
        if let Some(cancel) = self.sheet_cancel.take() {
            self.ui.perform(&request(Action::Click, cancel.into()));
        }
    }
}
