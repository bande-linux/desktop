use std::cell::{Cell as Flag, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use accesskit::{ActionRequest, Node, Role, TreeUpdate};
use unbound_files::{Place, Row as SidebarRow};
use unbound_fs::{Entry, SortBy, format_date, format_size};
use unbound_toolkit::ui::{
    Alert, Axis, Button, Cell, Column, Context, Cursor, Event, Key, Label, PopUpButton, Row, Size,
    Sort, Stack, TEXT_FIELD_HEIGHT, Table, TextField, Ui, Widget, WidgetId,
};
use unbound_toolkit::{Appearance, Image, Item, Rect, Scene, Shape, TextSystem};
use unbound_xdg::{OCTET_STREAM, UserDir, config_home, find_icon, user_dir};
use xdg_desktop_portal_unbound::file_chooser::{Mode, Selection};
use xdg_desktop_portal_unbound::settings::icon_theme;

use crate::L10N;
use crate::model::{Accept, Chooser};

const SIDEBAR_WIDTH: f32 = 184.0;
const BAR_HEIGHT: f32 = 44.0;
const FOOTER_HEIGHT: f32 = 56.0;
const NAME_WIDTH: f32 = 300.0;
const MARGIN: f32 = 16.0;
const GAP: f32 = 8.0;
const ICON_PIXELS: u32 = 16;
const PROMPT_WIDTH: f32 = 320.0;
const PLACE_PREFIX: &str = "place:";

#[derive(Debug, Clone, PartialEq)]
enum Intent {
    Go(PathBuf),
    Select(Vec<String>),
    Activate(Vec<String>),
    Name(String),
    Filter(usize),
    Sort(Sort),
    NewFolder,
    CreateFolder(String),
    AskGoTo(String),
    GoTo(String),
    Replace(PathBuf),
    Accept,
    Cancel,
}

type Intents = Rc<RefCell<Vec<Intent>>>;

fn push(intents: &Intents, intent: Intent) {
    intents.borrow_mut().push(intent);
}

fn label(text: &str) -> String {
    text.replace('_', "")
}

struct Slots {
    sidebar: usize,
    path: usize,
    files: usize,
    tools: usize,
    buttons: usize,
    name: Option<(usize, usize)>,
}

struct Layout {
    id: WidgetId,
    children: Vec<Box<dyn Widget>>,
    slots: Slots,
    frame: Rect,
}

impl Layout {
    fn top(&self) -> f32 {
        match self.slots.name {
            Some(_) => 2.0 * BAR_HEIGHT,
            None => BAR_HEIGHT,
        }
    }

    fn place_child(&mut self, cx: &mut Context, index: usize, frame: Rect) {
        if let Some(child) = self.children.get_mut(index) {
            child.measure(cx, Size::new(frame.width, frame.height));
            child.place(cx, frame);
        }
    }

    fn measure_child(&mut self, cx: &mut Context, index: usize, available: Size) -> Size {
        self.children
            .get_mut(index)
            .map_or(Size::new(0.0, 0.0), |child| child.measure(cx, available))
    }
}

impl Widget for Layout {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let content = frame.x + SIDEBAR_WIDTH;
        let width = (frame.width - SIDEBAR_WIDTH).max(0.0);
        let top = self.top();
        let centred = |row: f32, height: f32| frame.y + row + ((BAR_HEIGHT - height) / 2.0).round();
        self.place_child(
            cx,
            self.slots.sidebar,
            Rect::new(
                frame.x + GAP,
                frame.y + GAP,
                SIDEBAR_WIDTH - 2.0 * GAP,
                frame.height - GAP,
            ),
        );
        if let Some((name_label, name)) = self.slots.name {
            let size = self.measure_child(cx, name_label, Size::new(width, BAR_HEIGHT));
            let field_x = content + MARGIN + size.width + GAP;
            self.place_child(
                cx,
                name_label,
                Rect::new(
                    content + MARGIN,
                    centred(0.0, size.height),
                    size.width,
                    size.height,
                ),
            );
            self.place_child(
                cx,
                name,
                Rect::new(
                    field_x,
                    centred(0.0, TEXT_FIELD_HEIGHT),
                    NAME_WIDTH.min((content + width - MARGIN - field_x).max(0.0)),
                    TEXT_FIELD_HEIGHT,
                ),
            );
        }
        let path = self.measure_child(
            cx,
            self.slots.path,
            Size::new(width - 2.0 * MARGIN, BAR_HEIGHT),
        );
        self.place_child(
            cx,
            self.slots.path,
            Rect::new(
                content + MARGIN,
                centred(top - BAR_HEIGHT, path.height),
                (width - 2.0 * MARGIN).max(0.0),
                path.height,
            ),
        );
        self.place_child(
            cx,
            self.slots.files,
            Rect::new(
                content,
                frame.y + top,
                width,
                (frame.height - top - FOOTER_HEIGHT).max(0.0),
            ),
        );
        let footer = frame.y + frame.height - FOOTER_HEIGHT;
        let buttons = self.measure_child(cx, self.slots.buttons, Size::new(width, FOOTER_HEIGHT));
        self.place_child(
            cx,
            self.slots.buttons,
            Rect::new(
                content + width - MARGIN - buttons.width,
                footer + ((FOOTER_HEIGHT - buttons.height) / 2.0).round(),
                buttons.width,
                buttons.height,
            ),
        );
        let room = (width - 3.0 * MARGIN - buttons.width).max(0.0);
        let tools = self.measure_child(cx, self.slots.tools, Size::new(room, FOOTER_HEIGHT));
        self.place_child(
            cx,
            self.slots.tools,
            Rect::new(
                content + MARGIN,
                footer + ((FOOTER_HEIGHT - tools.height) / 2.0).round(),
                tools.width.min(room),
                tools.height,
            ),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let frame = self.frame;
        let palette = cx.palette;
        let content = Rect::new(
            frame.x + SIDEBAR_WIDTH,
            frame.y,
            (frame.width - SIDEBAR_WIDTH).max(0.0),
            frame.height,
        );
        items.push(Item::Fill {
            shape: Shape::rect(frame),
            paint: palette.chassis.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(content),
            paint: palette.sheet.into(),
        });
        let line = |x: f32, y: f32, width: f32, height: f32| Item::Fill {
            shape: Shape::rect(Rect::new(x, y, width, height)),
            paint: palette.hairline.into(),
        };
        items.push(line(content.x, frame.y, 1.0, frame.height));
        items.push(line(
            content.x,
            frame.y + self.top() - 1.0,
            content.width,
            1.0,
        ));
        items.push(line(
            content.x,
            frame.y + frame.height - FOOTER_HEIGHT,
            content.width,
            1.0,
        ));
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        Node::new(Role::GenericContainer)
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

struct Prompt {
    id: WidgetId,
    question: String,
    children: Vec<Box<dyn Widget>>,
    done: Rc<Flag<bool>>,
    frame: Rect,
}

impl Prompt {
    fn new(
        intents: &Intents,
        question: &str,
        text: &str,
        accept: &str,
        intent: fn(String) -> Intent,
    ) -> Self {
        let done = Rc::new(Flag::new(false));
        let typed = Rc::new(RefCell::new(text.to_owned()));
        let changed = Rc::clone(&typed);
        let (cancel_done, accept_done, accepted) =
            (Rc::clone(&done), Rc::clone(&done), Rc::clone(intents));
        let buttons: Vec<Box<dyn Widget>> = vec![
            Box::new(
                Button::new(&L10N.tr("button-cancel"), move || cancel_done.set(true)).cancel(),
            ),
            Box::new(
                Button::new(accept, move || {
                    accept_done.set(true);
                    push(&accepted, intent(typed.borrow().clone()));
                })
                .primary(),
            ),
        ];
        Self {
            id: WidgetId::next(),
            question: question.to_owned(),
            children: vec![
                Box::new(Label::new(question)),
                Box::new(TextField::new("", text, move |text: &str| {
                    *changed.borrow_mut() = text.to_owned()
                })),
                Box::new(Stack::new(Axis::Horizontal, GAP, 0.0, buttons)),
            ],
            done,
            frame: Rect::default(),
        }
    }

    fn new_folder(intents: &Intents) -> Self {
        Self::new(
            intents,
            &L10N.tr("new-folder-prompt"),
            &L10N.tr("untitled-folder"),
            &L10N.tr("button-create"),
            Intent::CreateFolder,
        )
    }

    fn go_to(intents: &Intents, text: &str) -> Self {
        Self::new(
            intents,
            &L10N.tr("go-to-prompt"),
            text,
            &L10N.tr("button-go"),
            Intent::GoTo,
        )
    }
}

impl Widget for Prompt {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, _available: Size) -> Size {
        let inner = Size::new(PROMPT_WIDTH - 2.0 * MARGIN, BAR_HEIGHT);
        let height: f32 = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, inner).height)
            .sum();
        Size::new(PROMPT_WIDTH, height + 2.0 * MARGIN + 2.0 * GAP + GAP)
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let inner = Size::new(frame.width - 2.0 * MARGIN, BAR_HEIGHT);
        let mut y = frame.y + MARGIN;
        let last = self.children.len().saturating_sub(1);
        for (index, child) in self.children.iter_mut().enumerate() {
            let size = child.measure(cx, inner);
            let x = match index == last {
                true => frame.x + frame.width - MARGIN - size.width,
                false => frame.x + MARGIN,
            };
            let width = match index == last {
                true => size.width,
                false => inner.width,
            };
            child.place(cx, Rect::new(x, y, width, size.height));
            y += size.height + GAP + if index + 1 == last { GAP } else { 0.0 };
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        for child in &self.children {
            child.paint(cx, items);
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        node.set_label(self.question.clone());
        node
    }

    fn dismissed(&self) -> bool {
        self.done.get()
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

pub struct Icons {
    data_dirs: Vec<PathBuf>,
    themes: Vec<String>,
    loaded: HashMap<String, Option<Image>>,
}

impl Icons {
    pub fn new(data_dirs: Vec<PathBuf>) -> Self {
        Self {
            data_dirs,
            themes: Vec::new(),
            loaded: HashMap::new(),
        }
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        let themes = vec![
            icon_theme(appearance.accent),
            "Unbound".to_owned(),
            "Adwaita".to_owned(),
        ];
        if themes != self.themes {
            self.themes = themes;
            self.loaded.clear();
        }
    }

    fn first(&mut self, names: &[String]) -> Option<Image> {
        let themes: Vec<&str> = self.themes.iter().map(String::as_str).collect();
        names.iter().find_map(|name| {
            self.loaded
                .entry(name.clone())
                .or_insert_with(|| {
                    find_icon(name, ICON_PIXELS, &themes, &self.data_dirs)
                        .and_then(|path| Image::load(&path, ICON_PIXELS))
                })
                .clone()
        })
    }
}

pub enum Finished {
    Chosen(Selection),
    Cancelled,
}

pub struct ChooserView {
    layout: WidgetId,
    ui: Ui,
    chooser: Chooser,
    places: Vec<SidebarRow>,
    home: PathBuf,
    icons: Icons,
    intents: Intents,
    files: WidgetId,
    focus: WidgetId,
    selected: Vec<PathBuf>,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    finished: Option<Finished>,
}

fn place_key(place: &Place) -> Option<String> {
    match &place.location {
        unbound_files::Location::Folder(path) => Some(format!("{PLACE_PREFIX}{}", path.display())),
        _ => None,
    }
}

fn folder_places(rows: Vec<SidebarRow>) -> Vec<SidebarRow> {
    let mut kept: Vec<SidebarRow> = Vec::new();
    for row in rows {
        match &row {
            SidebarRow::Heading(_) => {
                if matches!(kept.last(), Some(SidebarRow::Heading(_))) {
                    kept.pop();
                }
                kept.push(row);
            }
            SidebarRow::Place(place) if place_key(place).is_some() => kept.push(row),
            SidebarRow::Place(_) => {}
        }
    }
    if matches!(kept.last(), Some(SidebarRow::Heading(_))) {
        kept.pop();
    }
    kept
}

impl ChooserView {
    pub fn new(
        chooser: Chooser,
        places: Vec<SidebarRow>,
        home: PathBuf,
        icons: Icons,
        appearance: Appearance,
        text: TextSystem,
    ) -> Self {
        let layout = Layout {
            id: WidgetId::next(),
            children: Vec::new(),
            slots: Slots {
                sidebar: 0,
                path: 1,
                files: 2,
                tools: 3,
                buttons: 4,
                name: None,
            },
            frame: Rect::default(),
        };
        let mut view = Self {
            layout: layout.id,
            ui: Ui::new(Box::new(layout), Context::new(text, appearance.palette())),
            chooser,
            places: folder_places(places),
            home,
            icons,
            intents: Rc::new(RefCell::new(Vec::new())),
            files: WidgetId::next(),
            focus: WidgetId::next(),
            selected: Vec::new(),
            back: Vec::new(),
            forward: Vec::new(),
            finished: None,
        };
        view.ui.set_appearance(appearance);
        view.icons.set_appearance(appearance);
        view.rebuild();
        view
    }

    fn accept_label(&self) -> String {
        let dialog = self.chooser.dialog();
        dialog.accept_label.as_deref().map_or_else(
            || {
                L10N.tr(match dialog.mode {
                    Mode::Open {
                        directory: false, ..
                    } => "button-open",
                    Mode::Open { .. } | Mode::SaveMany { .. } => "button-choose",
                    Mode::Save { .. } => "button-save",
                })
            },
            label,
        )
    }

    fn sidebar(&mut self) -> Table {
        let intents = Rc::clone(&self.intents);
        let current = format!("{PLACE_PREFIX}{}", self.chooser.folder().display());
        let rows: Vec<Row> = self
            .places
            .clone()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                SidebarRow::Heading(title) => Some(Row::new(
                    format!("heading:{index}"),
                    vec![Cell::new(title.as_str()).muted()],
                )),
                SidebarRow::Place(place) => place_key(place).map(|key| {
                    let icon = self.icons.first(&[place.icon.to_owned()]);
                    Row::new(key, vec![Cell::new(place.name.as_str()).icon(icon)])
                }),
            })
            .collect();
        let mut table = Table::new(vec![Column::new("", SIDEBAR_WIDTH).flexible()])
            .without_header()
            .on_selection(move |keys| {
                if let Some(path) = keys.first().and_then(|key| key.strip_prefix(PLACE_PREFIX)) {
                    push(&intents, Intent::Go(PathBuf::from(path)));
                }
            });
        table.set_rows(rows);
        table.select(&[current]);
        table
    }

    fn path_bar(&self) -> Stack {
        let mut children: Vec<Box<dyn Widget>> = Vec::new();
        let folder = self.chooser.folder().to_path_buf();
        let ancestors: Vec<&Path> = match folder.starts_with(&self.home) {
            true => folder
                .ancestors()
                .take_while(|ancestor| ancestor.starts_with(&self.home))
                .collect(),
            false => folder.ancestors().collect(),
        };
        for (index, ancestor) in ancestors.iter().rev().enumerate() {
            if index > 0 {
                children.push(Box::new(Label::new("›")));
            }
            let name = ancestor.file_name().map_or_else(
                || "/".to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            let intents = Rc::clone(&self.intents);
            let target = ancestor.to_path_buf();
            children.push(Box::new(
                Button::new(&name, move || push(&intents, Intent::Go(target.clone()))).quiet(),
            ));
        }
        let shown = children.len().saturating_sub(9);
        Stack::new(Axis::Horizontal, 2.0, 0.0, children.split_off(shown))
    }

    fn file_row(&mut self, entry: &Entry) -> Row {
        let icon = match entry.is_folder() {
            true => self.icons.first(&["folder".to_owned()]),
            false => {
                let mime = self
                    .chooser
                    .mime()
                    .for_name(&entry.name)
                    .unwrap_or(OCTET_STREAM)
                    .to_owned();
                let mut names = self.chooser.mime().icon_names(&mime);
                names.push("text-x-generic".to_owned());
                self.icons.first(&names)
            }
        };
        let size = match entry.is_folder() {
            true => Cell::new("—").muted(),
            false => Cell::new(format_size(entry.size)).muted(),
        };
        Row::new(
            entry.path.to_string_lossy().into_owned(),
            vec![
                Cell::new(entry.name.as_str()).icon(icon),
                Cell::new(entry.modified.map(format_date).unwrap_or_default()).muted(),
                size,
            ],
        )
    }

    fn file_rows(&mut self) -> Vec<Row> {
        let entries: Vec<Entry> = self.chooser.entries().into_iter().cloned().collect();
        entries.iter().map(|entry| self.file_row(entry)).collect()
    }

    fn files_table(&mut self) -> Table {
        let (select, activate, sort) = (
            Rc::clone(&self.intents),
            Rc::clone(&self.intents),
            Rc::clone(&self.intents),
        );
        let mut table = Table::new(vec![
            Column::new(&L10N.tr("column-name"), 260.0).flexible(),
            Column::new(&L10N.tr("column-modified"), 150.0),
            Column::new(&L10N.tr("column-size"), 80.0).numeric(),
        ])
        .on_selection(move |keys| push(&select, Intent::Select(keys.to_vec())))
        .on_activate(move |keys| push(&activate, Intent::Activate(keys.to_vec())))
        .on_sort(move |order| push(&sort, Intent::Sort(order)));
        let order = self.chooser.sort();
        table.set_sort(Some(Sort {
            column: match order.by {
                SortBy::Name | SortBy::Kind | SortBy::Created => 0,
                SortBy::Modified => 1,
                SortBy::Size => 2,
            },
            ascending: !order.descending,
        }));
        let rows = self.file_rows();
        table.set_rows(rows);
        table
    }

    fn tools(&self) -> Stack {
        let mut tools: Vec<Box<dyn Widget>> = Vec::new();
        let filters: Vec<String> = self
            .chooser
            .filters()
            .iter()
            .map(|filter| filter.name.clone())
            .collect();
        if filters.len() > 1 {
            let intents = Rc::clone(&self.intents);
            tools.push(Box::new(PopUpButton::new(
                &L10N.tr("filter"),
                &filters,
                self.chooser.filter().unwrap_or(0),
                move |index| push(&intents, Intent::Filter(index)),
            )));
        }
        if !matches!(
            self.chooser.dialog().mode,
            Mode::Open {
                directory: false,
                ..
            }
        ) {
            let intents = Rc::clone(&self.intents);
            tools.push(Box::new(Button::new(&L10N.tr("new-folder"), move || {
                push(&intents, Intent::NewFolder)
            })));
        }
        Stack::new(Axis::Horizontal, GAP, 0.0, tools)
    }

    fn buttons(&self) -> Stack {
        let (cancel, accept) = (Rc::clone(&self.intents), Rc::clone(&self.intents));
        Stack::new(
            Axis::Horizontal,
            GAP,
            0.0,
            vec![
                Box::new(
                    Button::new(&L10N.tr("button-cancel"), move || {
                        push(&cancel, Intent::Cancel)
                    })
                    .cancel(),
                ),
                Box::new(
                    Button::new(&self.accept_label(), move || push(&accept, Intent::Accept))
                        .primary(),
                ),
            ],
        )
    }

    fn rebuild(&mut self) {
        let sidebar = self.sidebar();
        let files = self.files_table();
        self.files = files.id();
        self.focus = self.files;
        let mut children: Vec<Box<dyn Widget>> = vec![
            Box::new(sidebar),
            Box::new(self.path_bar()),
            Box::new(files),
            Box::new(self.tools()),
            Box::new(self.buttons()),
        ];
        let name = match self.chooser.dialog().mode {
            Mode::Save { .. } => {
                let field = self.name_field();
                self.focus = field.id();
                children.push(Box::new(Label::new(&L10N.tr("save-as"))));
                children.push(Box::new(field));
                Some((5, 6))
            }
            _ => None,
        };
        let slots = Slots {
            sidebar: 0,
            path: 1,
            files: 2,
            tools: 3,
            buttons: 4,
            name,
        };
        if let Some(layout) = self.ui.widget_as::<Layout>(self.layout) {
            layout.children = children;
            layout.slots = slots;
        }
        self.ui.set_focus(self.focus);
        self.select_stem(self.focus);
        self.selected.clear();
    }

    fn name_field(&self) -> TextField {
        let intents = Rc::clone(&self.intents);
        TextField::new(
            &L10N.tr("name-placeholder"),
            &self.chooser.name,
            move |text: &str| push(&intents, Intent::Name(text.to_owned())),
        )
    }

    fn select_stem(&mut self, field: WidgetId) {
        let stem = match self.chooser.name.rfind(".") {
            Some(dot) if dot > 0 => dot,
            _ => self.chooser.name.len(),
        };
        if let Some(field) = self.ui.widget_as::<TextField>(field) {
            field.select(0..stem);
        }
    }

    fn show_name(&mut self) {
        let field = self.name_field();
        let id = field.id();
        let Some(layout) = self.ui.widget_as::<Layout>(self.layout) else {
            return;
        };
        let Some(slot) = layout
            .slots
            .name
            .and_then(|(_, name)| layout.children.get_mut(name))
        else {
            return;
        };
        *slot = Box::new(field);
        self.select_stem(id);
    }

    fn go(&mut self, folder: PathBuf) {
        let left = self.chooser.folder().to_path_buf();
        if self.open(folder) {
            self.back.push(left);
            self.forward.clear();
        }
    }

    fn open(&mut self, folder: PathBuf) -> bool {
        if folder == self.chooser.folder() {
            return false;
        }
        match self.chooser.open(&folder) {
            Ok(()) => {
                self.rebuild();
                true
            }
            Err(error) => {
                tracing::warn!("cannot open {}: {error}", folder.display());
                false
            }
        }
    }

    fn step(&mut self, back: bool) {
        let popped = match back {
            true => self.back.pop(),
            false => self.forward.pop(),
        };
        let Some(folder) = popped else {
            return;
        };
        let left = self.chooser.folder().to_path_buf();
        if !self.open(folder) {
            return;
        }
        if left.parent() == Some(self.chooser.folder()) {
            self.select(std::slice::from_ref(&left));
        }
        match back {
            true => self.forward.push(left),
            false => self.back.push(left),
        }
    }

    fn go_to(&mut self, typed: &str) {
        let path = unbound_fs::typed_location(typed, &self.home, self.chooser.folder());
        if path.is_dir() {
            self.go(path);
        } else if let Some(parent) = path.parent().filter(|_| path.exists()) {
            self.go(parent.to_path_buf());
            self.select(&[path]);
        } else {
            let alert = Alert::new(
                &L10N.tr_args(
                    "go-to-missing",
                    [("path", path.display().to_string().into())],
                ),
                &L10N.tr("go-to-missing-message"),
                |_| {},
            )
            .primary(&L10N.tr("button-ok"), "ok");
            self.ui.present_sheet(alert);
        }
    }

    fn user_folder(&self, dir: UserDir) -> PathBuf {
        let user_dirs = config_home()
            .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok());
        user_dir(&self.home, user_dirs.as_deref(), dir)
    }

    fn shown_folder(&self) -> String {
        let folder = self.chooser.folder();
        match folder.strip_prefix(&self.home) {
            Ok(inside) if inside.as_os_str().is_empty() => "~/".to_owned(),
            Ok(inside) => format!("~/{}/", inside.display()),
            Err(_) if folder.parent().is_none() => "/".to_owned(),
            Err(_) => format!("{}/", folder.display()),
        }
    }

    fn accept(&mut self) {
        match self.chooser.accept(&self.selected) {
            Accept::Done(selection) => self.finished = Some(Finished::Chosen(selection)),
            Accept::Enter(folder) => self.go(folder),
            Accept::Replace(path) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let intents = Rc::clone(&self.intents);
                let alert = Alert::new(
                    &L10N.tr_args("replace-title", [("name", name.into())]),
                    &L10N.tr("replace-message"),
                    move |action| {
                        if action == "replace" {
                            push(&intents, Intent::Replace(path.clone()));
                        }
                    },
                )
                .cancel(&L10N.tr("button-cancel"), "cancel")
                .destructive(&L10N.tr("button-replace"), "replace");
                self.ui.present_sheet(alert);
            }
            Accept::Nothing => {}
        }
    }

    fn create_folder(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            return;
        }
        let folder = self.chooser.folder().join(name);
        match std::fs::create_dir(&folder) {
            Ok(()) => self.go(folder),
            Err(error) => tracing::warn!("cannot create {}: {error}", folder.display()),
        }
    }

    fn drain(&mut self) {
        loop {
            let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
            if intents.is_empty() {
                return;
            }
            for intent in intents {
                match intent {
                    Intent::Go(folder) => self.go(folder),
                    Intent::Select(keys) => {
                        self.selected = self.paths(&keys);
                        if let (Mode::Save { .. }, [only]) =
                            (&self.chooser.dialog().mode, self.selected.as_slice())
                            && !only.is_dir()
                            && let Some(name) = only.file_name()
                        {
                            self.chooser.name = name.to_string_lossy().into_owned();
                            self.show_name();
                        }
                    }
                    Intent::Activate(keys) => {
                        self.selected = self.paths(&keys);
                        match self.selected.as_slice() {
                            [only] if only.is_dir() => self.go(only.clone()),
                            _ => self.accept(),
                        }
                    }
                    Intent::Name(name) => self.chooser.name = name,
                    Intent::Filter(index) => {
                        self.chooser.set_filter(index);
                        self.rebuild();
                    }
                    Intent::Sort(order) => {
                        self.chooser.set_sort(unbound_fs::Sort {
                            by: match order.column {
                                1 => SortBy::Modified,
                                2 => SortBy::Size,
                                _ => SortBy::Name,
                            },
                            descending: !order.ascending,
                        });
                        let rows = self.file_rows();
                        if let Some(table) = self.ui.widget_as::<Table>(self.files) {
                            table.set_sort(Some(order));
                            table.set_rows(rows);
                        }
                    }
                    Intent::NewFolder => self.ui.present_sheet(Prompt::new_folder(&self.intents)),
                    Intent::AskGoTo(text) => {
                        self.ui.present_sheet(Prompt::go_to(&self.intents, &text));
                    }
                    Intent::GoTo(typed) => self.go_to(&typed),
                    Intent::CreateFolder(name) => self.create_folder(&name),
                    Intent::Replace(path) => {
                        self.finished = Some(Finished::Chosen(self.chooser.replace(path)))
                    }
                    Intent::Accept => self.accept(),
                    Intent::Cancel => self.finished = Some(Finished::Cancelled),
                }
            }
        }
    }

    fn paths(&self, keys: &[String]) -> Vec<PathBuf> {
        let entries = self.chooser.entries();
        let by_key: HashMap<_, _> = entries
            .iter()
            .map(|entry| (entry.path.to_string_lossy(), &entry.path))
            .collect();
        keys.iter()
            .map(|key| {
                by_key
                    .get(key.as_str())
                    .map_or_else(|| PathBuf::from(key), |path| (*path).clone())
            })
            .collect()
    }

    fn select(&mut self, paths: &[PathBuf]) {
        let keys: Vec<String> = paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        if let Some(table) = self.ui.widget_as::<Table>(self.files) {
            table.select(&keys);
        }
        self.selected = paths.to_vec();
    }

    fn shortcut(&mut self, key: Key, logo: bool, shift: bool) -> bool {
        if self.ui.has_sheet() {
            return false;
        }
        if !logo {
            return match key {
                Key::Character(typed @ ('/' | '~')) if self.ui.focus() == Some(self.files) => {
                    push(&self.intents, Intent::AskGoTo(typed.to_string()));
                    self.drain();
                    true
                }
                _ => false,
            };
        }
        match key {
            Key::Up => {
                if let Some(parent) = self.chooser.folder().parent() {
                    let child = self.chooser.folder().to_path_buf();
                    self.go(parent.to_path_buf());
                    self.select(&[child]);
                }
                true
            }
            Key::Down => {
                if let [only] = self.selected.as_slice()
                    && only.is_dir()
                {
                    self.go(only.clone());
                }
                true
            }
            Key::Character('[') if !shift => {
                self.step(true);
                true
            }
            Key::Character(']') if !shift => {
                self.step(false);
                true
            }
            Key::Character(letter) if shift => {
                match letter.to_ascii_lowercase() {
                    'n' => push(&self.intents, Intent::NewFolder),
                    'g' => push(&self.intents, Intent::AskGoTo(self.shown_folder())),
                    'h' => push(&self.intents, Intent::Go(self.home.clone())),
                    'd' => push(
                        &self.intents,
                        Intent::Go(self.user_folder(UserDir::Desktop)),
                    ),
                    'o' => push(
                        &self.intents,
                        Intent::Go(self.user_folder(UserDir::Documents)),
                    ),
                    _ => return false,
                }
                self.drain();
                true
            }
            _ => false,
        }
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::Key { key, modifiers } = event
            && self.shortcut(*key, modifiers.logo, modifiers.shift)
        {
            return true;
        }
        let handled = self.ui.handle(event);
        self.drain();
        handled
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        let performed = self.ui.perform(request);
        self.drain();
        performed
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
        self.icons.set_appearance(appearance);
        let selected = self.selected.clone();
        self.rebuild();
        self.select(&selected);
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.ui.scene(width, height)
    }

    pub fn cursor(&self) -> Cursor {
        self.ui.cursor()
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn deadline(&self) -> Option<std::time::Instant> {
        self.ui.deadline()
    }

    pub fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    pub fn folder(&self) -> PathBuf {
        self.chooser.folder().to_path_buf()
    }

    pub fn take_finished(&mut self) -> Option<Finished> {
        self.finished.take()
    }
}
