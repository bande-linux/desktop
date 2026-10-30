use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Action, ActionRequest, TreeId};
use unbound_fs::{Sort, SortBy, format_date, format_size};
use unbound_toolkit::ui::{
    Alert, Cell, ClipboardRequest, Column, Event, FreeLayout, HEADER_HEIGHT, Key, Modifiers,
    Pointer, PointerButton, Row, SearchField, Size, Sort as TableSort, Table, TextField, Toggle,
    Widget, WidgetId,
};
use unbound_toolkit::{Image, Point, Rect};

use crate::L10N;
use crate::prompt::{Kind, Prompt};
use crate::view::{
    COLUMN_WIDTH, FilesView, ROW_HEIGHT, Rename, Request, SEARCH_HEIGHT, SLOW_CLICK_RENAME, Search,
    Sheet, contains,
};

const NAME_SIZE: f32 = 13.0;
const TABLE_SORTS: [SortBy; 3] = [SortBy::Name, SortBy::Modified, SortBy::Size];
const ROW_INSET: f32 = 6.0;
const NAME_INSET: f32 = 28.0;
const TABLE_TEXT_INSET: f32 = 6.0 + 16.0 + 6.0;
const FIELD_INSET: f32 = 3.0;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Signal {
    Choice(String),
    Selection(Vec<String>),
    Activate,
    Sort(TableSort),
    Search(String),
    GoTo(String),
    ApplyToAll(bool),
    Drag(Point),
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
}

fn sender(signals: &Signals) -> impl Fn(Signal) + 'static {
    let signals = signals.clone();
    move |signal| signals.borrow_mut().push(signal)
}

pub(crate) fn table(signals: &Signals) -> Table {
    let (selection, activate, sort, drag) = (
        sender(signals),
        sender(signals),
        sender(signals),
        sender(signals),
    );
    Table::new(vec![
        Column::new(&L10N.tr("column-name"), 280.0),
        Column::new(&L10N.tr("column-modified"), 170.0),
        Column::new(&L10N.tr("column-size"), 80.0).numeric(),
        Column::new(&L10N.tr("column-kind"), 160.0).unsortable(),
    ])
    .on_selection(move |keys| selection(Signal::Selection(keys.to_vec())))
    .on_activate(move |_| activate(Signal::Activate))
    .on_sort(move |order| sort(Signal::Sort(order)))
    .on_drag(move |_, at| drag(Signal::Drag(at)))
}

pub(crate) fn key_of(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn type_key(name: &str, folder: bool) -> String {
    match (folder, name.split_once('.')) {
        (true, _) => String::new(),
        (false, Some((_, suffix))) => suffix.to_lowercase(),
        (false, None) => name.to_lowercase(),
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

    fn focus(&mut self, id: WidgetId) {
        self.ui.perform(&request(Action::Focus, id.into()));
    }

    pub(crate) fn focus_content(&mut self) {
        match self.browser.single() {
            true => self.focus(self.table),
            false => self.ui.clear_focus(),
        }
    }

    fn insert(&mut self, widget: impl Widget + 'static) -> WidgetId {
        let id = widget.id();
        if let Some(host) = self.host_mut() {
            host.insert(widget, Rect::default());
        }
        self.layout_controls();
        id
    }

    fn remove(&mut self, id: WidgetId) {
        if let Some(host) = self.host_mut() {
            host.remove(id);
        }
    }

    fn search_focused(&self) -> bool {
        self.search
            .as_ref()
            .is_some_and(|search| self.ui.focus() == Some(search.field))
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
        if self.browser.single()
            && table_key(key, modifiers)
            && self.ui.handle(&Event::Key { key, modifiers })
        {
            return true;
        }
        if self.key(key, modifiers) {
            return true;
        }
        self.browser.single() && key != Key::Tab && self.ui.handle(&Event::Key { key, modifiers })
    }

    fn over_control(&mut self, at: Point) -> bool {
        let ids = [Some(self.table)]
            .into_iter()
            .chain([self.search.as_ref().map(|search| search.field)])
            .chain([self.rename.as_ref().map(|rename| rename.field)])
            .flatten()
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
        if let Some(field) = self.search.as_ref().map(|search| search.field)
            && self
                .frame_of(field)
                .is_some_and(|frame| contains(frame, at))
        {
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
        self.ui.take_menu_request();
        match self.ui.take_clipboard_request() {
            Some(ClipboardRequest::Copy(text)) => self.requests.push(Request::CopyText(text)),
            Some(ClipboardRequest::Paste) => self.requests.push(Request::Paste),
            None => {}
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
                    Signal::Selection(keys) => self.table_selected(&keys),
                    Signal::Activate => self.open(),
                    Signal::Sort(order) => self.table_sorted(order),
                    Signal::Search(text) => self.apply_search(&text),
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

    fn table_selected(&mut self, keys: &[String]) {
        if !self.browser.single() {
            return;
        }
        let rows = keys
            .iter()
            .filter_map(|key| self.table_keys.get(key).copied())
            .collect();
        self.browser.mark(0, rows);
        self.table_revealed = self.browser.active_column().selected;
        self.selection_changed();
    }

    fn table_sorted(&mut self, order: TableSort) {
        let Some(by) = TABLE_SORTS.get(order.column).copied() else {
            return;
        };
        self.browser.set_sort(Sort {
            by,
            descending: !order.ascending,
        });
        self.selection_changed();
    }

    fn table(&mut self) -> Option<&mut Table> {
        self.ui.widget_as::<Table>(self.table)
    }

    pub(crate) fn shown_table(&self) -> Option<&Table> {
        self.ui.widget_ref::<Table>(self.table)
    }

    fn table_contents(&mut self) -> (Vec<Row>, HashMap<String, usize>) {
        let entries = std::mem::take(&mut self.browser.columns_mut()[0].entries);
        let mut types: HashMap<String, (Option<Image>, String)> = HashMap::new();
        let mut keys = HashMap::with_capacity(entries.len());
        let mut rows = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let folder = entry.is_folder();
            let (icon, kind) = match types.get(&type_key(&entry.name, folder)) {
                Some(known) => known.clone(),
                None => {
                    let known = (
                        self.entry_icon(entry, 32),
                        self.apps.kind_for_name(&entry.name, folder),
                    );
                    types.insert(type_key(&entry.name, folder), known.clone());
                    known
                }
            };
            let key = key_of(&entry.path);
            keys.insert(key.clone(), index);
            let size = match folder {
                true => "\u{2014}".to_owned(),
                false => format_size(entry.size),
            };
            rows.push(Row::new(
                key,
                vec![
                    Cell::new(entry.name.clone()).icon(icon),
                    Cell::new(entry.modified.map(format_date).unwrap_or_default()).muted(),
                    Cell::new(size).muted(),
                    Cell::new(kind).muted(),
                ],
            ));
        }
        self.browser.columns_mut()[0].entries = entries;
        (rows, keys)
    }

    pub(crate) fn sync_table(&mut self) {
        self.layout_controls();
        if !self.browser.single() {
            return;
        }
        let revision = self.browser.columns()[0].revision;
        if self.table_revision != Some(revision) {
            let (rows, keys) = self.table_contents();
            self.table_keys = keys;
            if let Some(table) = self.table() {
                table.set_rows(rows);
            }
            self.table_revision = Some(revision);
            self.table_revealed = None;
        }
        let sort = self.browser.sort();
        let column = &self.browser.columns()[0];
        let wanted = column
            .marked
            .iter()
            .filter_map(|row| column.entries.get(*row))
            .map(|entry| key_of(&entry.path))
            .collect::<Vec<_>>();
        let cursor = column.selected;
        let shown = TABLE_SORTS
            .iter()
            .position(|by| *by == sort.by)
            .map(|column| TableSort {
                column,
                ascending: !sort.descending,
            });
        if let Some(table) = self.table() {
            table.set_sort(shown);
            if table.selection() != wanted {
                table.select(&wanted);
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
        if self.browser.single() {
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
        let table = match self.browser.single() {
            true => content,
            false => Rect::default(),
        };
        let search = self.search.as_ref().map(|search| {
            let width = (content.width - 20.0).min(360.0);
            (
                search.field,
                Rect::new(content.x + 10.0, 7.0, width, SEARCH_HEIGHT - 14.0),
            )
        });
        let rename = self
            .rename
            .as_ref()
            .map(|rename| (rename.field, rename.path.clone()));
        let rename = rename.map(|(field, path)| (field, self.rename_frame(&path)));
        let table_id = self.table;
        if let Some(host) = self.host_mut() {
            host.set_frame(table_id, table);
            for (id, frame) in search.into_iter().chain(rename) {
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

    pub(crate) fn open_search(&mut self) {
        self.commit_rename();
        if let Some(field) = self.search.as_ref().map(|search| search.field) {
            self.focus(field);
            return;
        }
        let send = sender(&self.signals);
        let field = SearchField::new(&L10N.tr("search-placeholder"), "", move |text| {
            send(Signal::Search(text.to_owned()));
        });
        let column = self.browser.active();
        let field = self.insert(field);
        self.search = Some(Search { column, field });
        self.layout_controls();
        self.focus(field);
    }

    pub(crate) fn close_search(&mut self) {
        if let Some(search) = self.search.take() {
            self.remove(search.field);
            self.browser.set_filter(search.column, "");
            self.selection_changed();
            self.focus_content();
        }
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
