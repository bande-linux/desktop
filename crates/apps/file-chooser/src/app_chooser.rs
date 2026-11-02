use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::process::ExitCode;
use std::rc::Rc;

use accesskit::TreeUpdate;
use bande_file_chooser::L10N;
use bande_toolkit::ui::{
    Alert, Axis, Bounded, Cell, Column, Context, FreeLayout, Row, SearchField, Stack, Table, Ui,
    Widget, WidgetId,
};
use bande_toolkit::{
    App, Color, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, TextSystem,
    Window, WindowEvent,
};
use bande_xdg::{
    DesktopEntry, ICON_THEMES, Locale, current_desktops, data_dirs, find_icon, handlers,
};
use xdg_desktop_portal_bande::app_chooser::Offer;
use xdg_desktop_portal_bande::dialog_process::{CANCELLED, CHOSEN, decode_choices, encode_pairs};

const OPEN: &str = "open";
const LIST_HEIGHT: f32 = 240.0;
const SPACING: f32 = 10.0;
const ICON_PIXELS: u32 = 32;
const FALLBACK_ICON: &str = "application-x-executable";
const UNAVAILABLE: u8 = 2;

enum Signal {
    Search(String),
    Open,
    Cancel,
}

type Signals = Rc<RefCell<Vec<Signal>>>;

fn push(signals: &Signals, signal: Signal) {
    signals.borrow_mut().push(signal);
}

struct Chooser {
    ui: Ui,
    table: WidgetId,
    signals: Signals,
    entries: Vec<DesktopEntry>,
    choices: Vec<String>,
    search: String,
    icons: HashMap<String, Option<Image>>,
    stdin: Option<File>,
    pending: Vec<u8>,
    chosen: Option<Option<(String, Option<String>)>>,
}

impl Chooser {
    fn icon(&mut self, name: Option<&str>) -> Option<Image> {
        let name = name.unwrap_or(FALLBACK_ICON).to_owned();
        let dirs = data_dirs();
        self.icons
            .entry(name.clone())
            .or_insert_with(|| {
                [name.as_str(), FALLBACK_ICON].into_iter().find_map(|name| {
                    find_icon(name, ICON_PIXELS, &ICON_THEMES, &dirs)
                        .and_then(|path| Image::load(&path, ICON_PIXELS))
                })
            })
            .clone()
    }

    fn rows(&mut self) -> Vec<Row> {
        let search = self.search.trim().to_lowercase();
        let shown: Vec<(String, String, Option<String>)> = self
            .choices
            .iter()
            .filter_map(|choice| self.entries.iter().find(|entry| entry.app_id() == choice))
            .filter(|entry| search.is_empty() || entry.name.to_lowercase().contains(&search))
            .map(|entry| {
                (
                    entry.app_id().to_owned(),
                    entry.name.clone(),
                    entry.icon.clone(),
                )
            })
            .collect();
        shown
            .into_iter()
            .map(|(id, name, icon)| {
                let icon = self.icon(icon.as_deref());
                Row::new(id, vec![Cell::new(name.as_str()).icon(icon)])
            })
            .collect()
    }

    fn refresh(&mut self) {
        let rows = self.rows();
        let first = rows.first().map(|row| row.key.clone());
        if let Some(table) = self.ui.widget_as::<Table>(self.table) {
            let kept = table.selection();
            table.set_rows(rows);
            if table.selection().is_empty() || kept.is_empty() {
                table.select(&first.into_iter().collect::<Vec<_>>());
            }
        }
    }

    fn selected(&mut self) -> Option<String> {
        self.ui
            .widget_ref::<Table>(self.table)
            .and_then(|table| table.selection().into_iter().next())
    }

    fn read_updates(&mut self) -> bool {
        let Some(stdin) = &mut self.stdin else {
            return false;
        };
        let mut buffer = [0u8; 4096];
        let read = stdin.read(&mut buffer);
        match read {
            Ok(0) | Err(_) => {
                self.stdin = None;
                return false;
            }
            Ok(count) => self.pending.extend_from_slice(&buffer[..count]),
        }
        let mut updated = false;
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            self.choices = decode_choices(&line[..end]);
            updated = true;
        }
        if updated {
            if self
                .choices
                .iter()
                .any(|choice| !self.entries.iter().any(|entry| entry.app_id() == choice))
            {
                self.entries = installed();
            }
            self.refresh();
        }
        updated
    }
}

impl App for Chooser {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let mut scene = self.ui.scene(width, height);
        scene.background = Color::TRANSPARENT;
        scene
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(input) => self.ui.handle(&input),
            WindowEvent::Accessibility(request) => self.ui.perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                true
            }
            _ => false,
        };
        let signals: Vec<Signal> = self.signals.borrow_mut().drain(..).collect();
        for signal in signals {
            match signal {
                Signal::Search(text) => {
                    self.search = text;
                    self.refresh();
                }
                Signal::Open => {
                    if self.chosen.is_none()
                        && let Some(app) = self.selected()
                    {
                        let token = window.activation_token(&app).ok();
                        self.chosen = Some(Some((app, token)));
                    }
                }
                Signal::Cancel => {
                    if self.chosen.is_none() {
                        self.chosen = Some(None);
                    }
                }
            }
        }
        if changed {
            window.request_redraw();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.stdin.iter().map(|stdin| stdin.as_fd()).collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        if self.read_updates() {
            window.request_redraw();
        }
    }

    fn finished(&self) -> bool {
        self.chosen.is_some() && !self.ui.is_animating()
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        if self.chosen.is_none() {
            self.chosen = Some(None);
        }
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

fn installed() -> Vec<DesktopEntry> {
    handlers(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    )
}

fn shown_name(location: &str) -> String {
    let trimmed = location.trim_end_matches('/');
    let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
    bande_xdg::percent_decode(name)
        .to_string_lossy()
        .into_owned()
}

fn alert(offer: &Offer, signals: &Signals, choosing: bool) -> Alert {
    let name = offer
        .location
        .as_deref()
        .map(shown_name)
        .filter(|name| !name.is_empty());
    let (title, message) = match (choosing, name) {
        (true, Some(name)) => (
            L10N.tr_args("choose-app-title", [("name", name.into())]),
            String::new(),
        ),
        (true, None) => (L10N.tr("choose-app-title-unnamed"), String::new()),
        (false, Some(name)) => (
            L10N.tr_args("choose-app-none", [("name", name.into())]),
            L10N.tr("choose-app-none-message"),
        ),
        (false, None) => (
            L10N.tr("choose-app-none-unnamed"),
            L10N.tr("choose-app-none-message"),
        ),
    };
    let answers = Rc::clone(signals);
    let alert = Alert::new(&title, &message, move |action| {
        push(
            &answers,
            match action {
                OPEN => Signal::Open,
                _ => Signal::Cancel,
            },
        )
    });
    match choosing {
        true => alert
            .cancel(&L10N.tr("button-cancel"), "cancel")
            .primary(&L10N.tr("button-open"), OPEN),
        false => alert.primary(&L10N.tr("button-ok"), "cancel"),
    }
}

pub fn choose(offer: &Offer) -> ExitCode {
    let mut window = match Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "app-chooser".to_owned(),
        }),
        (0, 0),
    ) {
        Ok(window) => window,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::from(UNAVAILABLE);
        }
    };
    let appearance = window.appearance();
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    let signals = Signals::default();
    let (activate, search) = (Rc::clone(&signals), Rc::clone(&signals));
    let table = Table::new(vec![Column::new(&L10N.tr("column-name"), 300.0).flexible()])
        .without_header()
        .on_activate(move |_| push(&activate, Signal::Open));
    let table_id = table.id();
    let field = SearchField::new(&L10N.tr("search-placeholder"), "", move |text| {
        push(&search, Signal::Search(text.to_owned()))
    });
    let field_id = field.id();
    let entries = installed();
    let choosing = offer
        .choices
        .iter()
        .any(|choice| entries.iter().any(|entry| entry.app_id() == choice));
    let mut dialog = alert(offer, &signals, choosing);
    if choosing {
        dialog = dialog.accessory(Stack::new(
            Axis::Vertical,
            SPACING,
            0.0,
            vec![Box::new(field), Box::new(Bounded::new(table, LIST_HEIGHT))],
        ));
    }
    ui.present_dialog(dialog);
    if choosing {
        ui.set_focus(field_id);
    }
    let mut chooser = Chooser {
        ui,
        table: table_id,
        signals,
        entries,
        choices: offer.choices.clone(),
        search: String::new(),
        icons: HashMap::new(),
        stdin: std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .map(File::from),
        pending: Vec::new(),
        chosen: None,
    };
    chooser.refresh();
    if let Some(last) = &offer.last_choice
        && let Some(table) = chooser.ui.widget_as::<Table>(table_id)
        && table.row_count() > 0
    {
        let current = table.selection();
        table.select(std::slice::from_ref(last));
        if table.selection().is_empty() {
            table.select(&current);
        }
    }
    if let Err(error) = bande_toolkit::run(&mut window, &mut chooser) {
        tracing::error!("{error}");
        return ExitCode::from(UNAVAILABLE);
    }
    let Some(Some((app, token))) = chooser.chosen else {
        return ExitCode::from(CANCELLED);
    };
    let mut stdout = std::io::stdout();
    match stdout
        .write_all(&encode_pairs(&[(app, token.unwrap_or_default())]))
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::from(CHOSEN),
        Err(error) => {
            tracing::error!("cannot hand the choice to the portal: {error}");
            ExitCode::from(UNAVAILABLE)
        }
    }
}
