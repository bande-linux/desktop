use std::cell::RefCell;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead, Read, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::mpsc::{Sender, channel};
use std::time::Instant;

use accesskit::{Node, Role as AccessRole, TreeUpdate};
use bande_content::pdf::protocol::{Render, Use};
use bande_content::pdf::session::{Change, Phase, Session};
use bande_inbox::{Inbox, Outbox};
use bande_l10n::tr;
use bande_pdf::{Orientation, PageSetup, Paper, keep_pages};
use bande_printers::{Capabilities, JobOptions, PrintClient, PrinterInfo};
use bande_toolkit::ui::{
    Button, Context, Form, FormLine, FreeLayout, Label, PopUpButton, SegmentedControl, Size,
    Stepper, TextField, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    App, Color, Elevation, Image, Item, Palette, Paragraph, Point, Rect, Role, Scene, Shape,
    TextAlign, TextStyle, TextSystem, Window, WindowEvent,
};
use xdg_desktop_portal_bande::dialog_process::{
    CANCELLED, CHOSEN, DialogProcess, encode_pairs, parse_print,
};
use xdg_desktop_portal_bande::file_chooser::{Dialog, FileChooserUi, Mode, Outcome};
use xdg_desktop_portal_bande::print::{DocumentSource, PrintAnswer, PrintRequest};
use xdg_desktop_portal_bande::request::Cancellation;
use zbus::blocking::Connection;

const APP_ID: &str = "org.bande.PrintDialog";
const UNAVAILABLE: u8 = 2;
const WAYLAND_PARENT: &str = "wayland:";
const WINDOW_SIZE: (u32, u32) = (860, 560);
const MARGIN: f32 = 20.0;
const PANEL_WIDTH: f32 = 380.0;
const BUTTON_ROW: f32 = 48.0;
const NAV_HEIGHT: f32 = 32.0;
const NAV_BUTTON_WIDTH: f32 = 36.0;
const SHEET_RADIUS: f32 = 2.0;
const TWO_SIDED_CHOICES: [&str; 3] = ["one-sided", "two-sided-long-edge", "two-sided-short-edge"];
const LARGEST_COPIES: f64 = 999.0;
const HINT_SIZE: f32 = 13.0;

#[derive(Debug)]
enum Signal {
    Printer(usize),
    Copies(i32),
    Pages(String),
    Paper(usize),
    Landscape(bool),
    Sides(usize),
    Color(bool),
    Page(isize),
    Print,
    SaveAsPdf,
    Cancel,
}

type Signals = Rc<RefCell<Vec<Signal>>>;

fn push(signals: &Signals, signal: Signal) {
    signals.borrow_mut().push(signal);
}

enum Arrival {
    Printers(Vec<PrinterInfo>),
    Capabilities(String, Capabilities),
    Laid(u64, PathBuf),
    LayoutFailed,
    Submitted(Result<(), String>),
    Saved(Result<bool, String>),
}

enum Source {
    Document(PathBuf),
    App { requests: Sender<(u64, PageSetup)> },
    Later,
}

#[derive(Default)]
struct Sheet {
    media: (f32, f32),
    image: Option<Image>,
    page: Option<(f32, f32)>,
    hint: String,
    caption: String,
}

struct SheetPreview {
    id: WidgetId,
    frame: Rect,
    hint: Option<Paragraph>,
    sheet: Rc<RefCell<Sheet>>,
}

fn fitted(content: (f32, f32), area: Rect) -> Rect {
    let scale = (area.width / content.0).min(area.height / content.1);
    let (width, height) = (content.0 * scale, content.1 * scale);
    Rect::new(
        area.x + (area.width - width) / 2.0,
        area.y + (area.height - height) / 2.0,
        width,
        height,
    )
}

impl Widget for SheetPreview {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, _cx: &mut Context, available: Size) -> Size {
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let sheet = self.sheet.borrow();
        self.hint = (!sheet.hint.is_empty() && sheet.media.0 > 0.0).then(|| {
            let paper = fitted(sheet.media, frame);
            cx.text.paragraph(
                &sheet.hint,
                TextStyle::new(HINT_SIZE, Color::BLACK.with_opacity(0.55)),
                Some(paper.width - 2.0 * MARGIN),
                TextAlign::Center,
            )
        });
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let sheet = self.sheet.borrow();
        if sheet.media.0 <= 0.0 || sheet.media.1 <= 0.0 {
            return;
        }
        let paper = fitted(sheet.media, self.frame);
        let shape = Shape::rounded(paper, SHEET_RADIUS);
        items.extend(Item::shadows(
            shape,
            Elevation::Overlay,
            cx.palette.scheme,
            true,
        ));
        items.push(Item::Fill {
            shape,
            paint: Color::WHITE.into(),
        });
        if let (Some(image), Some(page)) = (&sheet.image, sheet.page) {
            items.push(Item::Image {
                rect: fitted(page, paper),
                image: image.clone(),
            });
        }
        if let Some(paragraph) = &self.hint {
            items.push(Item::Text {
                origin: Point::new(
                    paper.x + MARGIN,
                    paper.y + (paper.height - paragraph.height()) / 2.0,
                ),
                paragraph: paragraph.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(AccessRole::Image);
        node.set_label(self.sheet.borrow().caption.clone());
        node
    }
}

struct Dialogue {
    ui: Ui,
    signals: Signals,
    inbox: Inbox<Arrival>,
    source: Source,
    session: Option<Session>,
    session_file: Option<PathBuf>,
    generation: u64,
    title: String,
    app_id: String,
    window_handle: Option<String>,
    printers: Vec<PrinterInfo>,
    printer: Option<String>,
    capabilities: Capabilities,
    media_keywords: Vec<String>,
    media: String,
    landscape: bool,
    sides: String,
    color: bool,
    copies: i32,
    pages_text: String,
    pages: Vec<(f32, f32)>,
    shown_page: usize,
    sheet: Rc<RefCell<Sheet>>,
    scale: f32,
    printer_popup: WidgetId,
    paper_popup: WidgetId,
    sides_popup: WidgetId,
    color_control: WidgetId,
    print_button: WidgetId,
    nav_label: WidgetId,
    message: WidgetId,
    busy: bool,
    answer: Option<ExitCode>,
    prepared: Option<PrintAnswer>,
}

fn media_name(keyword: &str) -> String {
    match keyword {
        "iso_a4_210x297mm" => tr!("A4"),
        "iso_a3_297x420mm" => tr!("A3"),
        "iso_a5_148x210mm" => tr!("A5"),
        "na_letter_8.5x11in" => tr!("US Letter"),
        "na_legal_8.5x14in" => tr!("US Legal"),
        _ => keyword
            .rsplit('_')
            .next()
            .map_or_else(|| keyword.to_owned(), str::to_owned),
    }
}

fn side_name(side: &str) -> String {
    match side {
        "two-sided-long-edge" => tr!("Long edge"),
        "two-sided-short-edge" => tr!("Short edge"),
        _ => tr!("Off"),
    }
}

pub fn parse_ranges(text: &str) -> Option<Vec<(i32, i32)>> {
    let mut ranges = Vec::new();
    for part in text
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (first, last) = part.split_once('-').unwrap_or((part, part));
        let first: i32 = first.trim().parse().ok().filter(|page| *page > 0)?;
        let last: i32 = last.trim().parse().ok().filter(|page| *page >= first)?;
        ranges.push((first, last));
    }
    Some(ranges)
}

fn page_setup(media: &str, landscape: bool) -> PageSetup {
    PageSetup {
        paper: Paper::with_keyword(media).unwrap_or(Paper::A4),
        orientation: match landscape {
            true => Orientation::Landscape,
            false => Orientation::Portrait,
        },
        ..PageSetup::default()
    }
}

impl Dialogue {
    fn options(&self) -> Option<JobOptions> {
        Some(JobOptions {
            media: self.media.clone(),
            landscape: self.landscape,
            sides: self.sides.clone(),
            color: self.color,
            copies: self.copies,
            pages: parse_ranges(&self.pages_text)?,
        })
    }

    fn refresh_printers(&mut self) {
        let choices: Vec<String> = match self.printers.is_empty() {
            true => vec![tr!("No printers found")],
            false => self
                .printers
                .iter()
                .map(|printer| match printer.reachable {
                    true => printer.name.clone(),
                    false => {
                        tr!("{name} (offline)", name = printer.name.clone())
                    }
                })
                .collect(),
        };
        let current = self
            .printers
            .iter()
            .position(|printer| self.printer.as_ref() == Some(&printer.id));
        let selected = current
            .or_else(|| self.printers.iter().position(|printer| printer.default))
            .or_else(|| self.printers.iter().position(|printer| printer.reachable))
            .unwrap_or(0);
        if let Some(popup) = self.ui.widget_as::<PopUpButton>(self.printer_popup) {
            popup.set_choices(&choices, selected);
        }
        let chosen = self
            .printers
            .get(selected)
            .map(|printer| printer.id.clone());
        if chosen != self.printer {
            self.printer = chosen;
            self.ask_capabilities();
        }
        self.refresh_print_button();
    }

    fn refresh_print_button(&mut self) {
        let can = self.printer.is_some()
            && !self.busy
            && parse_ranges(&self.pages_text).is_some()
            && !matches!(
                self.session.as_ref().map(Session::phase),
                Some(Phase::Failed)
            );
        if let Some(button) = self.ui.widget_as::<Button>(self.print_button) {
            button.set_enabled(can);
        }
    }

    fn ask_capabilities(&self) {
        let Some(printer) = self.printer.clone() else {
            return;
        };
        let outbox = self.inbox.outbox();
        std::thread::spawn(move || {
            let answer = Connection::session()
                .and_then(|session| PrintClient::new(&session)?.capabilities(&printer));
            match answer {
                Ok(capabilities) => {
                    outbox.send(Arrival::Capabilities(printer, capabilities));
                }
                Err(error) => tracing::info!("cannot ask {printer} what it can do: {error}"),
            }
        });
    }

    fn apply_capabilities(&mut self) {
        let known: Vec<String> = self
            .capabilities
            .media
            .iter()
            .filter(|keyword| Paper::with_keyword(keyword).is_some())
            .cloned()
            .collect();
        self.media_keywords = match known.is_empty() {
            true => Paper::ALL
                .iter()
                .map(|paper| paper.keyword.to_owned())
                .collect(),
            false => known,
        };
        let default = self.capabilities.default_media.clone();
        if !self.media_keywords.contains(&self.media) {
            self.media = match self.media_keywords.contains(&default) {
                true => default,
                false => self.media_keywords[0].clone(),
            };
            self.lay_out();
        }
        let names: Vec<String> = self
            .media_keywords
            .iter()
            .map(|key| media_name(key))
            .collect();
        let selected = self
            .media_keywords
            .iter()
            .position(|keyword| *keyword == self.media)
            .unwrap_or(0);
        if let Some(popup) = self.ui.widget_as::<PopUpButton>(self.paper_popup) {
            popup.set_choices(&names, selected);
        }
        let two_sided = self.capabilities.two_sided;
        let sides: Vec<String> = TWO_SIDED_CHOICES
            .iter()
            .take(if two_sided { 3 } else { 1 })
            .map(|side| side_name(side))
            .collect();
        if !two_sided {
            self.sides = TWO_SIDED_CHOICES[0].to_owned();
        }
        let chosen_side = TWO_SIDED_CHOICES
            .iter()
            .position(|side| *side == self.sides)
            .unwrap_or(0);
        if let Some(popup) = self.ui.widget_as::<PopUpButton>(self.sides_popup) {
            popup.set_choices(&sides, chosen_side);
        }
        let has_color = self.capabilities.color;
        if !has_color {
            self.color = false;
        }
        let selected_color = usize::from(!self.color);
        self.ui
            .update_widget::<SegmentedControl>(self.color_control, |control, cx| {
                control.set_selected(cx, selected_color);
                control.set_enabled(has_color);
            });
    }

    fn refresh_sheet(&mut self) {
        let setup = page_setup(&self.media, self.landscape);
        let mut sheet = self.sheet.borrow_mut();
        sheet.media = setup.size();
        sheet.page = self.pages.get(self.shown_page).copied();
        let total = self.pages.len();
        sheet.caption = match total {
            0 => tr!("No preview"),
            _ => tr!(
                "Page {page} of {total}",
                page = (self.shown_page + 1),
                total = total
            ),
        };
        sheet.hint = match self.source {
            Source::Later => tr!("The app prepares its pages after you press Print"),
            _ => String::new(),
        };
        let caption = sheet.caption.clone();
        drop(sheet);
        if let Some(label) = self.ui.widget_as::<Label>(self.nav_label) {
            label.set_text(&caption);
        }
    }

    fn render_shown_page(&mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let Some(page) = self.pages.get(self.shown_page).copied() else {
            return;
        };
        let setup = page_setup(&self.media, self.landscape);
        let area = Rect::new(
            0.0,
            0.0,
            WINDOW_SIZE.0 as f32 - PANEL_WIDTH - 3.0 * MARGIN,
            WINDOW_SIZE.1 as f32 - BUTTON_ROW - NAV_HEIGHT - 3.0 * MARGIN,
        );
        let paper = fitted(setup.size(), area);
        let target = fitted(page, paper);
        let scale = target.width / page.0 * self.scale;
        session.want(vec![Render {
            page: self.shown_page,
            scale,
            x: 0,
            y: 0,
            width: (page.0 * scale).ceil() as u32,
            height: (page.1 * scale).ceil() as u32,
            purpose: Use::Page,
        }]);
    }

    fn open_document(&mut self, path: PathBuf) {
        if let Some(old) = self.session_file.take()
            && matches!(self.source, Source::App { .. })
        {
            let _ = std::fs::remove_file(old);
        }
        self.pages.clear();
        self.sheet.borrow_mut().image = None;
        match Session::open(&path) {
            Ok(session) => self.session = Some(session),
            Err(error) => {
                tracing::warn!("cannot read the document to preview it: {error}");
                self.session = None;
            }
        }
        self.session_file = Some(path);
        self.refresh_sheet();
    }

    fn lay_out(&mut self) {
        self.generation += 1;
        if let Source::App { requests } = &self.source {
            let _ = requests.send((self.generation, page_setup(&self.media, self.landscape)));
        }
        self.refresh_sheet();
    }

    fn set_message(&mut self, text: &str) {
        if let Some(label) = self.ui.widget_as::<Label>(self.message) {
            label.set_text(text);
        }
    }

    fn print(&mut self) {
        let Some(options) = self.options() else {
            return;
        };
        let Some(printer) = self.printer.clone() else {
            return;
        };
        if matches!(self.source, Source::Later) {
            self.prepared = Some(PrintAnswer { printer, options });
            self.answer = Some(ExitCode::from(CHOSEN));
            return;
        }
        let Some(path) = self.session_file.clone() else {
            return;
        };
        self.busy = true;
        self.refresh_print_button();
        let title = self.title.clone();
        let outbox = self.inbox.outbox();
        std::thread::spawn(move || {
            let submitted = File::open(&path)
                .map(OwnedFd::from)
                .map_err(|error| error.to_string())
                .and_then(|document| {
                    Connection::session()
                        .and_then(|session| {
                            PrintClient::new(&session)?.submit(&title, &printer, &options, document)
                        })
                        .map(drop)
                        .map_err(|error| error.to_string())
                });
            outbox.send(Arrival::Submitted(submitted));
        });
    }

    fn save_as_pdf(&mut self) {
        let Some(path) = self.session_file.clone() else {
            return;
        };
        let Some(ranges) = parse_ranges(&self.pages_text) else {
            return;
        };
        let name = format!("{}.pdf", self.title.trim_end_matches(".pdf"));
        let dialog = Dialog {
            app_id: self.app_id.clone(),
            parent_window: self
                .window_handle
                .as_ref()
                .map_or_else(String::new, |handle| format!("{WAYLAND_PARENT}{handle}")),
            title: tr!("Save as PDF"),
            accept_label: None,
            modal: true,
            mode: Mode::Save {
                name: Some(name),
                file: None,
            },
            filters: Vec::new(),
            current_filter: None,
            choices: Vec::new(),
            folder: None,
        };
        let pages = self.pages.len();
        let outbox = self.inbox.outbox();
        std::thread::spawn(move || {
            outbox.send(Arrival::Saved(save(&path, &dialog, &ranges, pages)));
        });
    }

    fn take_signals(&mut self) {
        let signals: Vec<Signal> = self.signals.borrow_mut().drain(..).collect();
        for signal in signals {
            match signal {
                Signal::Printer(index) => {
                    if let Some(printer) = self.printers.get(index) {
                        self.printer = Some(printer.id.clone());
                        self.ask_capabilities();
                    }
                }
                Signal::Copies(copies) => self.copies = copies,
                Signal::Pages(text) => {
                    self.pages_text = text;
                }
                Signal::Paper(index) => {
                    if let Some(media) = self.media_keywords.get(index) {
                        self.media.clone_from(media);
                        self.lay_out();
                    }
                }
                Signal::Landscape(landscape) => {
                    self.landscape = landscape;
                    self.lay_out();
                }
                Signal::Sides(index) => {
                    if let Some(side) = TWO_SIDED_CHOICES.get(index) {
                        (*side).clone_into(&mut self.sides);
                    }
                }
                Signal::Color(color) => self.color = color,
                Signal::Page(step) => {
                    let last = self.pages.len().saturating_sub(1);
                    self.shown_page = self.shown_page.saturating_add_signed(step).min(last);
                    self.refresh_sheet();
                    self.render_shown_page();
                }
                Signal::Print => self.print(),
                Signal::SaveAsPdf => self.save_as_pdf(),
                Signal::Cancel => self.answer = Some(ExitCode::from(CANCELLED)),
            }
        }
        self.refresh_print_button();
    }

    fn take_arrivals(&mut self) {
        for arrival in self.inbox.drain() {
            match arrival {
                Arrival::Printers(printers) => {
                    self.printers = printers;
                    self.refresh_printers();
                }
                Arrival::Capabilities(printer, capabilities) => {
                    if self.printer.as_ref() == Some(&printer) {
                        self.capabilities = capabilities;
                        self.apply_capabilities();
                    }
                }
                Arrival::Laid(generation, path) => {
                    if generation == self.generation {
                        self.open_document(path);
                    } else {
                        let _ = std::fs::remove_file(path);
                    }
                }
                Arrival::LayoutFailed => {
                    self.set_message(&tr!("The app could not lay out its pages."))
                }
                Arrival::Submitted(Ok(())) => self.answer = Some(ExitCode::from(CHOSEN)),
                Arrival::Submitted(Err(error)) => {
                    tracing::warn!("cannot hand the document to the print service: {error}");
                    self.busy = false;
                    self.set_message(&tr!(
                        "The document could not be handed to the print service."
                    ));
                }
                Arrival::Saved(Ok(true)) => self.answer = Some(ExitCode::from(CHOSEN)),
                Arrival::Saved(Ok(false)) => {}
                Arrival::Saved(Err(error)) => {
                    tracing::warn!("cannot save the PDF: {error}");
                    self.set_message(&tr!("The PDF could not be saved."));
                }
            }
        }
        self.refresh_print_button();
    }

    fn take_session(&mut self) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let mut failed = false;
        for change in session.take() {
            match change {
                Change::Opened => {
                    self.pages = session.opened().pages.clone();
                    self.shown_page = self.shown_page.min(self.pages.len().saturating_sub(1));
                }
                Change::Pixels(render, pixels) if render.page == self.shown_page => {
                    self.sheet.borrow_mut().image =
                        Image::from_rgba(render.width, render.height, pixels);
                }
                Change::Failed | Change::Locked => failed = true,
                _ => {}
            }
        }
        if failed {
            self.set_message(&tr!("No preview"));
        }
        self.refresh_sheet();
        if self.sheet.borrow().image.is_none() {
            self.render_shown_page();
        }
        self.refresh_print_button();
    }
}

fn save(path: &Path, dialog: &Dialog, ranges: &[(i32, i32)], pages: usize) -> Result<bool, String> {
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let process = DialogProcess { program };
    let outcome = process.choose(dialog, &Cancellation::default());
    let Outcome::Chosen(selection) = outcome else {
        return Ok(false);
    };
    let Some(target) = selection.paths.first() else {
        return Ok(false);
    };
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let kept: Vec<usize> = (0..pages)
        .filter(|page| {
            let number = *page as i32 + 1;
            ranges
                .iter()
                .any(|(first, last)| number >= *first && number <= *last)
        })
        .collect();
    let output = match ranges.is_empty() || kept.len() == pages {
        true => bytes,
        false => keep_pages(&bytes, &kept).map_err(|error| error.to_string())?,
    };
    bande_xdg::replace_file(target, output).map_err(|error| error.to_string())?;
    Ok(true)
}

impl App for Dialogue {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::Input(input) => self.ui.handle(&input),
            WindowEvent::Accessibility(request) => self.ui.perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                true
            }
            WindowEvent::CloseRequested => {
                self.answer = Some(ExitCode::from(CANCELLED));
                window.close();
                false
            }
            _ => false,
        };
        self.take_signals();
        if self.window_handle.is_none() {
            self.window_handle = window.exported_handle().map(str::to_owned);
        }
        self.scale = window.scale() as f32;
        if changed {
            window.request_redraw();
        }
        if self.answer.is_some() {
            window.close();
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        let mut sources = vec![self.inbox.fd()];
        if let Some(session) = &self.session {
            sources.push(session.fd());
        }
        sources
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        match index {
            0 => self.take_arrivals(),
            _ => self.take_session(),
        }
        window.request_redraw();
        if self.answer.is_some() {
            window.close();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, window: &mut Window, _now: Instant) {
        window.request_redraw();
    }

    fn finished(&self) -> bool {
        self.answer.is_some()
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

fn follow_printers(outbox: Outbox<Arrival>) {
    std::thread::spawn(move || {
        let followed = (|| -> zbus::Result<()> {
            let session = Connection::session()?;
            let client = PrintClient::new(&session)?;
            let changes = client.changes()?;
            client.watch()?;
            if !outbox.send(Arrival::Printers(client.printers()?)) {
                return Ok(());
            }
            for _ in changes {
                if !outbox.send(Arrival::Printers(client.printers()?)) {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(error) = followed {
            tracing::warn!("cannot reach the print service: {error}");
            outbox.send(Arrival::Printers(Vec::new()));
        }
    });
}

fn serve_layouts(outbox: Outbox<Arrival>) -> Sender<(u64, PageSetup)> {
    let (requests, incoming) = channel::<(u64, PageSetup)>();
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut stdout = std::io::stdout();
        let folder =
            std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
        while let Ok(mut latest) = incoming.recv() {
            while let Ok(newer) = incoming.try_recv() {
                latest = newer;
            }
            let (generation, setup) = latest;
            let laid = (|| -> std::io::Result<PathBuf> {
                writeln!(
                    stdout,
                    "{} {}",
                    setup.paper.keyword,
                    u8::from(setup.orientation == Orientation::Landscape)
                )?;
                stdout.flush()?;
                let mut line = String::new();
                stdin.read_line(&mut line)?;
                let length: u64 = line.trim().parse().map_err(std::io::Error::other)?;
                let path = folder.join(format!(
                    "bande-print-{}-{generation}.pdf",
                    std::process::id()
                ));
                let mut file = File::create_new(&path)?;
                std::io::copy(&mut (&mut stdin).take(length), &mut file)?;
                Ok(path)
            })();
            match laid {
                Ok(path) => {
                    outbox.send(Arrival::Laid(generation, path));
                }
                Err(error) => {
                    tracing::warn!("the app could not lay out its pages: {error}");
                    outbox.send(Arrival::LayoutFailed);
                }
            }
        }
    });
    requests
}

fn nav_button(symbol: &str, label: &str, signals: &Signals, step: isize) -> Button {
    let pressed = Rc::clone(signals);
    Button::new(symbol, move || push(&pressed, Signal::Page(step))).tooltip(label)
}

pub fn show(request: &PrintRequest) -> ExitCode {
    let title = tr!("Print “{title}”", title = request.title.clone());
    let mut window = match Window::open(Role::dialog(&title, APP_ID), WINDOW_SIZE) {
        Ok(window) => window,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::from(UNAVAILABLE);
        }
    };
    if let Some(handle) = request.parent_window.strip_prefix(WAYLAND_PARENT) {
        window.attach_to(handle);
    }
    window.export();
    let appearance = window.appearance();
    let signals = Signals::default();
    let inbox = match Inbox::new() {
        Ok(inbox) => inbox,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::from(UNAVAILABLE);
        }
    };
    let sheet = Rc::new(RefCell::new(Sheet::default()));
    let palette: Palette = appearance.palette();
    let mut root = FreeLayout::new();
    let (width, height) = (WINDOW_SIZE.0 as f32, WINDOW_SIZE.1 as f32);
    let preview_width = width - PANEL_WIDTH - 3.0 * MARGIN;
    let preview_height = height - BUTTON_ROW - NAV_HEIGHT - 3.0 * MARGIN;
    root.insert(
        SheetPreview {
            id: WidgetId::next(),
            frame: Rect::default(),
            hint: None,
            sheet: Rc::clone(&sheet),
        },
        Rect::new(MARGIN, MARGIN, preview_width, preview_height),
    );
    let nav_y = MARGIN + preview_height + MARGIN / 2.0;
    root.insert(
        nav_button("◀", &tr!("Previous page"), &signals, -1),
        Rect::new(MARGIN, nav_y, NAV_BUTTON_WIDTH, NAV_HEIGHT),
    );
    let nav_label = root.insert(
        Label::new("").secondary(),
        Rect::new(
            MARGIN + NAV_BUTTON_WIDTH + MARGIN / 2.0,
            nav_y,
            preview_width - 2.0 * (NAV_BUTTON_WIDTH + MARGIN),
            NAV_HEIGHT,
        ),
    );
    root.insert(
        nav_button("▶", &tr!("Next page"), &signals, 1),
        Rect::new(
            MARGIN + preview_width - NAV_BUTTON_WIDTH,
            nav_y,
            NAV_BUTTON_WIDTH,
            NAV_HEIGHT,
        ),
    );

    let on = |make: fn(usize) -> Signal| {
        let signals = Rc::clone(&signals);
        move |index: usize| push(&signals, make(index))
    };
    let printer_popup = PopUpButton::new(
        &tr!("Printer"),
        &[tr!("Looking for printers…")],
        0,
        on(Signal::Printer),
    );
    let printer_id = printer_popup.id();
    let copies_signals = Rc::clone(&signals);
    let copies = Stepper::new(
        &tr!("Copies"),
        1.0,
        (1.0, LARGEST_COPIES),
        1.0,
        move |value| {
            push(&copies_signals, Signal::Copies(value as i32));
        },
    );
    let pages_signals = Rc::clone(&signals);
    let pages = TextField::new(&tr!("All"), "", move |text| {
        push(&pages_signals, Signal::Pages(text.to_owned()));
    })
    .without_text_checking();
    let paper_popup = PopUpButton::new(
        &tr!("Paper"),
        &Paper::ALL
            .iter()
            .map(|paper| media_name(paper.keyword))
            .collect::<Vec<_>>(),
        0,
        on(Signal::Paper),
    );
    let paper_id = paper_popup.id();
    let landscape_signals = Rc::clone(&signals);
    let orientation = SegmentedControl::new(
        &tr!("Orientation"),
        &[tr!("Portrait"), tr!("Landscape")],
        Some(0),
        move |index| push(&landscape_signals, Signal::Landscape(index == 1)),
    );
    let sides_popup = PopUpButton::new(
        &tr!("Two-sided"),
        &[side_name(TWO_SIDED_CHOICES[0])],
        0,
        on(Signal::Sides),
    );
    let sides_id = sides_popup.id();
    let color_signals = Rc::clone(&signals);
    let color = SegmentedControl::new(
        &tr!("Color"),
        &[tr!("Color"), tr!("Black & White")],
        Some(0),
        move |index| push(&color_signals, Signal::Color(index == 0)),
    );
    let color_id = color.id();
    let form = Form::new(
        &tr!("Print options"),
        vec![
            FormLine::Row(tr!("Printer"), Box::new(printer_popup)),
            FormLine::Row(tr!("Copies"), Box::new(copies)),
            FormLine::Row(tr!("Pages"), Box::new(pages)),
            FormLine::Row(tr!("Paper"), Box::new(paper_popup)),
            FormLine::Row(tr!("Orientation"), Box::new(orientation)),
            FormLine::Row(tr!("Two-sided"), Box::new(sides_popup)),
            FormLine::Row(tr!("Color"), Box::new(color)),
        ],
    );
    let panel_x = width - PANEL_WIDTH - MARGIN;
    root.insert(
        form,
        Rect::new(panel_x, MARGIN, PANEL_WIDTH, preview_height),
    );
    let message = root.insert(
        Label::new("").secondary(),
        Rect::new(panel_x, nav_y, PANEL_WIDTH, NAV_HEIGHT),
    );
    let buttons_y = height - BUTTON_ROW - MARGIN / 2.0;
    let mut add_button = |button: Button, from_right: f32, wide: f32| {
        root.insert(
            button,
            Rect::new(
                width - MARGIN - from_right,
                buttons_y,
                wide,
                BUTTON_ROW - MARGIN / 2.0,
            ),
        )
    };
    let (print_signals, cancel_signals, pdf_signals) = (
        Rc::clone(&signals),
        Rc::clone(&signals),
        Rc::clone(&signals),
    );
    let print_button = add_button(
        Button::new(&tr!("Print"), move || push(&print_signals, Signal::Print)).primary(),
        100.0,
        100.0,
    );
    add_button(
        Button::new(&tr!("Cancel"), move || {
            push(&cancel_signals, Signal::Cancel)
        })
        .cancel(),
        210.0,
        100.0,
    );
    if !matches!(request.document, DocumentSource::Later) {
        root.insert(
            Button::new(&tr!("Save as PDF…"), move || {
                push(&pdf_signals, Signal::SaveAsPdf)
            }),
            Rect::new(MARGIN, buttons_y, 150.0, BUTTON_ROW - MARGIN / 2.0),
        );
    }
    let mut ui = Ui::new(Box::new(root), Context::new(TextSystem::new(), palette));
    ui.set_appearance(appearance);
    let source = match &request.document {
        DocumentSource::Later => Source::Later,
        DocumentSource::File(path) => Source::Document(path.clone()),
        DocumentSource::App => Source::App {
            requests: serve_layouts(inbox.outbox()),
        },
    };
    let initial = match &source {
        Source::Document(path) => Some(path.clone()),
        _ => None,
    };
    let scale = window.scale() as f32;
    let mut dialogue = Dialogue {
        ui,
        signals,
        inbox,
        source,
        session: None,
        session_file: None,
        generation: 0,
        title: request.title.clone(),
        app_id: request.app_id.clone(),
        window_handle: None,
        printers: Vec::new(),
        printer: None,
        capabilities: Capabilities::default(),
        media_keywords: Paper::ALL
            .iter()
            .map(|paper| paper.keyword.to_owned())
            .collect(),
        media: Paper::A4.keyword.to_owned(),
        landscape: false,
        sides: TWO_SIDED_CHOICES[0].to_owned(),
        color: true,
        copies: 1,
        pages_text: String::new(),
        pages: Vec::new(),
        shown_page: 0,
        sheet,
        scale,
        printer_popup: printer_id,
        paper_popup: paper_id,
        sides_popup: sides_id,
        color_control: color_id,
        print_button,
        nav_label,
        message,
        busy: false,
        answer: None,
        prepared: None,
    };
    follow_printers(dialogue.inbox.outbox());
    match initial {
        Some(path) => dialogue.open_document(path),
        None => dialogue.lay_out(),
    }
    let ran = bande_toolkit::run(&mut window, &mut dialogue);
    if let Some(path) = dialogue.session_file.take()
        && matches!(dialogue.source, Source::App { .. })
    {
        let _ = std::fs::remove_file(path);
    }
    if let Err(error) = ran {
        tracing::error!("{error}");
        return ExitCode::from(UNAVAILABLE);
    }
    if let Some(prepared) = &dialogue.prepared {
        let mut stdout = std::io::stdout();
        let bytes = encode_pairs(&prepared.pairs());
        if stdout
            .write_all(&bytes)
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return ExitCode::from(UNAVAILABLE);
        }
    }
    dialogue.answer.unwrap_or_else(|| ExitCode::from(CANCELLED))
}

pub fn run(arguments: Vec<OsString>) -> ExitCode {
    match parse_print(arguments) {
        Ok(request) => show(&request),
        Err(error) => {
            tracing::error!("{}", error.0);
            ExitCode::from(UNAVAILABLE)
        }
    }
}
