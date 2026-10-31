use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use unbound_fs::format_size;
use unbound_status::memory::{AppMemory, force_quit, heaviest_apps};
use unbound_toolkit::ui::{
    Alert, BUTTON_HEIGHT, Button, Cell, Column, Context, Event, FreeLayout, Key, Row, Size, Table,
    Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    App, Image, Item, Palette, Paragraph, Point, Radius, Rect, Scene, Shape, TextAlign, TextRole,
    TextStyle, TextSystem, Window, WindowEvent,
};
use unbound_xdg::{
    DesktopEntry, Locale, applications, current_desktops, data_dirs, find_icon, spawn_detached,
};
use zbus::blocking::Connection;

use crate::L10N;

pub const LOW_MEMORY_SIZE: (u32, u32) = (460, 410);
pub const ACTIVITY_MONITOR: &str = "org.unbound.ActivityMonitor";
const MONITOR_COMMAND: [&str; 2] = ["unbound-monitor", "--memory"];
const APPS_SHOWN: usize = 6;
const REFRESH: Duration = Duration::from_secs(2);
const PADDING: f32 = 20.0;
const DISC: f32 = 36.0;
const DISC_GAP: f32 = 14.0;
const TITLE_GAP: f32 = 6.0;
const LIST_GAP: f32 = 16.0;
const LIST_INSET: f32 = 4.0;
const MEMORY_COLUMN: f32 = 84.0;
const FOOTER: f32 = 60.0;
const FOOTER_INSET: f32 = 16.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 84.0;
const FORCE_QUIT: &str = "force-quit";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Signal {
    Selected(Vec<String>),
    ForceQuit,
    Chosen(String),
    ActivityMonitor,
    Close,
}

type Signals = Rc<RefCell<Vec<Signal>>>;

fn sender(signals: &Signals, signal: Signal) -> impl FnMut() + 'static {
    let signals = signals.clone();
    move || signals.borrow_mut().push(signal.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    ForceQuit,
    ActivityMonitor,
    Close,
}

struct Controls {
    layout: WidgetId,
    table: WidgetId,
    monitor: (WidgetId, f32),
    close: (WidgetId, f32),
    force_quit: (WidgetId, f32),
}

fn sized(mut button: Button, cx: &mut Context, layout: &mut FreeLayout) -> (WidgetId, f32) {
    let width = button
        .measure(cx, Size::new(f32::MAX, BUTTON_HEIGHT))
        .width
        .max(BUTTON_MIN_WIDTH);
    (layout.insert(button, Rect::default()), width)
}

pub struct LowMemoryView {
    ui: Ui,
    controls: Controls,
    signals: Signals,
    apps: Vec<AppMemory>,
    icons: HashMap<String, Image>,
    disc: Option<Image>,
    selected: Option<String>,
    confirming: Option<AppMemory>,
}

fn paragraph(text: &mut TextSystem, content: &str, style: TextStyle, width: f32) -> Paragraph {
    text.paragraph(content, style, Some(width), TextAlign::Start)
}

impl LowMemoryView {
    pub fn new(palette: Palette, text: TextSystem, disc: Option<Image>) -> Self {
        let signals = Signals::default();
        let mut layout = FreeLayout::new();
        let selection = signals.clone();
        let activation = signals.clone();
        let table = Table::new(vec![
            Column::new(&L10N.tr("low-memory-app"), 200.0).flexible(),
            Column::new(&L10N.tr("low-memory-used"), MEMORY_COLUMN).numeric(),
        ])
        .without_header()
        .on_selection(move |keys| selection.borrow_mut().push(Signal::Selected(keys.to_vec())))
        .on_activate(move |_| activation.borrow_mut().push(Signal::ForceQuit));
        let mut force_quit = Button::new(
            &L10N.tr("low-memory-force-quit"),
            sender(&signals, Signal::ForceQuit),
        )
        .destructive();
        force_quit.set_enabled(false);
        let mut cx = Context::new(text, palette);
        let table = layout.insert(table, Rect::default());
        let monitor = Button::new(
            &L10N.tr("low-memory-activity-monitor"),
            sender(&signals, Signal::ActivityMonitor),
        )
        .quiet();
        let close = Button::new(
            &L10N.tr("low-memory-close"),
            sender(&signals, Signal::Close),
        )
        .cancel();
        let controls = Controls {
            layout: layout.id(),
            table,
            monitor: sized(monitor, &mut cx, &mut layout),
            close: sized(close, &mut cx, &mut layout),
            force_quit: sized(force_quit, &mut cx, &mut layout),
        };
        Self {
            ui: Ui::new(Box::new(layout), cx),
            controls,
            signals,
            apps: Vec::new(),
            icons: HashMap::new(),
            disc,
            selected: None,
            confirming: None,
        }
    }

    pub fn set_apps(&mut self, apps: Vec<AppMemory>, icons: HashMap<String, Image>) {
        let rows = apps
            .iter()
            .map(|app| {
                let icon = app.icon.as_ref().and_then(|icon| icons.get(icon)).cloned();
                Row::new(
                    app.app_id.clone(),
                    vec![
                        Cell::new(app.name.as_str()).icon(icon),
                        Cell::new(format_size(app.bytes)).tabular(),
                    ],
                )
            })
            .collect();
        if let Some(table) = self.ui.widget_as::<Table>(self.controls.table) {
            table.set_rows(rows);
        }
        self.apps = apps;
        self.icons = icons;
        self.signals.borrow_mut().clear();
        let selected = self
            .selected
            .clone()
            .filter(|id| self.apps.iter().any(|app| app.app_id == *id));
        self.select(selected);
    }

    pub fn select(&mut self, app_id: Option<String>) {
        if let Some(table) = self.ui.widget_as::<Table>(self.controls.table) {
            table.select(app_id.as_slice());
        }
        self.signals.borrow_mut().clear();
        let enabled = app_id.is_some();
        self.selected = app_id;
        if let Some(button) = self.ui.widget_as::<Button>(self.controls.force_quit.0) {
            button.set_enabled(enabled);
        }
    }

    pub fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn place_controls(&mut self, list: Rect, width: f32, height: f32) {
        let y = height - FOOTER + ((FOOTER - BUTTON_HEIGHT) / 2.0).round();
        let (force_quit_id, force_quit) = self.controls.force_quit;
        let (close_id, close) = self.controls.close;
        let (monitor_id, monitor) = self.controls.monitor;
        let right = width - FOOTER_INSET;
        let frames = [
            (self.controls.table, list.inset(LIST_INSET)),
            (
                force_quit_id,
                Rect::new(right - force_quit, y, force_quit, BUTTON_HEIGHT),
            ),
            (
                close_id,
                Rect::new(
                    right - force_quit - BUTTON_GAP - close,
                    y,
                    close,
                    BUTTON_HEIGHT,
                ),
            ),
            (
                monitor_id,
                Rect::new(FOOTER_INSET, y, monitor, BUTTON_HEIGHT),
            ),
        ];
        if let Some(layout) = self.ui.widget_as::<FreeLayout>(self.controls.layout) {
            for (id, frame) in frames {
                layout.set_frame(id, frame);
            }
        }
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        let (width_points, height_points) = (width as f32, height as f32);
        let palette = self.ui.context().palette;
        let text_x = PADDING + DISC + DISC_GAP;
        let text_width = width_points - text_x - PADDING;
        let text = &mut self.ui.context().text;
        let heading = paragraph(
            text,
            &L10N.tr("low-memory-title"),
            TextStyle::role(TextRole::Heading, palette.ink),
            text_width,
        );
        let body = paragraph(
            text,
            &L10N.tr("low-memory-body"),
            TextStyle::role(TextRole::Body, palette.ink2),
            text_width,
        );
        let empty = self.apps.is_empty().then(|| {
            text.paragraph(
                &L10N.tr("low-memory-no-apps"),
                TextStyle::role(TextRole::Body, palette.ink2),
                Some(width_points - 2.0 * PADDING),
                TextAlign::Center,
            )
        });
        let body_top = PADDING + heading.height() + TITLE_GAP;
        let list_top = (body_top + body.height()).max(PADDING + DISC) + LIST_GAP;
        let list = Rect::new(
            PADDING,
            list_top.round(),
            width_points - 2.0 * PADDING,
            (height_points - FOOTER - LIST_GAP - list_top)
                .max(0.0)
                .round(),
        );
        self.place_controls(list, width_points, height_points);
        let mut items = Vec::new();
        if let Some(disc) = &self.disc {
            items.push(Item::Image {
                rect: Rect::new(PADDING, PADDING, DISC, DISC),
                image: disc.clone(),
            });
        }
        items.push(Item::Text {
            origin: Point::new(text_x, PADDING),
            paragraph: heading,
        });
        items.push(Item::Text {
            origin: Point::new(text_x, body_top),
            paragraph: body,
        });
        items.push(Item::Fill {
            shape: Shape::rounded(list, Radius::M),
            paint: palette.overlay.into(),
        });
        items.push(Item::Stroke {
            shape: Shape::rounded(list.inset(0.5), Radius::M - 0.5),
            width: 1.0,
            paint: palette.hairline.into(),
        });
        if let Some(empty) = empty {
            items.push(Item::Text {
                origin: Point::new(
                    PADDING,
                    (list.y + (list.height - empty.height()) / 2.0).round(),
                ),
                paragraph: empty,
            });
        }
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(0.0, height_points - FOOTER, width_points, FOOTER)),
            paint: palette.chassis.into(),
        });
        items.push(Item::Fill {
            shape: Shape::rect(Rect::new(0.0, height_points - FOOTER, width_points, 1.0)),
            paint: palette.hairline.into(),
        });
        let (controls, dialog) = self.ui.paint_layers(width, height);
        items.extend(controls);
        items.extend(dialog);
        Scene {
            width,
            height,
            background: palette.sheet,
            items,
        }
    }

    fn confirm(&mut self) {
        let Some(app) = self
            .selected
            .as_ref()
            .and_then(|id| self.apps.iter().find(|app| app.app_id == *id))
            .cloned()
        else {
            return;
        };
        let signals = self.signals.clone();
        let alert = Alert::new(
            &L10N.tr_args("low-memory-confirm", [("app", app.name.clone().into())]),
            &L10N.tr("low-memory-confirm-body"),
            move |choice| signals.borrow_mut().push(Signal::Chosen(choice.to_owned())),
        )
        .destructive(&L10N.tr("low-memory-force-quit"), FORCE_QUIT)
        .cancel(&L10N.tr("button-cancel"), "cancel");
        let alert = match app.icon.as_ref().and_then(|icon| self.icons.get(icon)) {
            Some(icon) => alert.icon(icon.clone()),
            None => alert,
        };
        self.confirming = Some(app);
        self.ui.present_dialog(alert);
    }

    pub fn handle(&mut self, event: &Event) -> (bool, Vec<(Effect, Option<AppMemory>)>) {
        let mut handled = match event {
            Event::Key {
                key: Key::Escape, ..
            } if !self.ui.has_sheet() => {
                self.signals.borrow_mut().push(Signal::Close);
                true
            }
            Event::Key {
                key: Key::Enter, ..
            } if !self.ui.has_sheet() && self.selected.is_some() => {
                self.signals.borrow_mut().push(Signal::ForceQuit);
                true
            }
            _ => self.ui.handle(event),
        };
        let mut effects = Vec::new();
        loop {
            let signals = std::mem::take(&mut *self.signals.borrow_mut());
            if signals.is_empty() {
                break;
            }
            handled = true;
            for signal in signals {
                match signal {
                    Signal::Selected(keys) => self.select(keys.into_iter().next()),
                    Signal::ForceQuit => self.confirm(),
                    Signal::Chosen(choice) => {
                        let app = self.confirming.take();
                        if choice == FORCE_QUIT && app.is_some() {
                            effects.push((Effect::ForceQuit, app));
                        }
                    }
                    Signal::ActivityMonitor => effects.push((Effect::ActivityMonitor, None)),
                    Signal::Close => effects.push((Effect::Close, None)),
                }
            }
        }
        (handled, effects)
    }
}

fn load_icons(apps: &[AppMemory], known: &mut HashMap<String, Image>, pixels: u32) {
    let directories = data_dirs();
    for icon in apps.iter().filter_map(|app| app.icon.as_ref()) {
        if known.contains_key(icon) {
            continue;
        }
        if let Some(image) =
            find_icon(icon, pixels, &[], &directories).and_then(|path| Image::load(&path, pixels))
        {
            known.insert(icon.clone(), image);
        }
    }
}

pub struct LowMemory {
    view: LowMemoryView,
    app_slice: Option<PathBuf>,
    entries: Vec<DesktopEntry>,
    session: Option<Connection>,
    icons: HashMap<String, Image>,
    pixels: u32,
    refreshed: Instant,
}

impl LowMemory {
    pub fn new(
        window: &Window,
        text: TextSystem,
        session: Option<Connection>,
        app_slice: Option<PathBuf>,
    ) -> Self {
        let pixels = |points: f32| (f64::from(points) * window.scale().max(2.0)).round() as u32;
        let disc_pixels = pixels(DISC);
        let disc = find_icon(ACTIVITY_MONITOR, disc_pixels, &[], &data_dirs())
            .and_then(|path| Image::load(&path, disc_pixels));
        let mut view = LowMemoryView::new(window.appearance().palette(), text, disc);
        view.ui().set_appearance(window.appearance());
        let mut low_memory = Self {
            view,
            app_slice,
            entries: applications(
                &data_dirs(),
                Locale::from_env().as_ref(),
                &current_desktops(),
            ),
            session,
            icons: HashMap::new(),
            pixels: disc_pixels,
            refreshed: Instant::now(),
        };
        low_memory.refresh(Instant::now());
        low_memory
    }

    fn refresh(&mut self, now: Instant) {
        self.refreshed = now;
        let apps = self
            .app_slice
            .as_deref()
            .map(|slice| heaviest_apps(slice, &self.entries, APPS_SHOWN))
            .unwrap_or_default();
        load_icons(&apps, &mut self.icons, self.pixels);
        self.view.set_apps(apps, self.icons.clone());
    }

    fn apply(&mut self, window: &mut Window, effects: Vec<(Effect, Option<AppMemory>)>) {
        for (effect, app) in effects {
            match (effect, app) {
                (Effect::ForceQuit, Some(app)) => {
                    let quit = self
                        .session
                        .as_ref()
                        .ok_or_else(|| zbus::Error::Failure("no session bus".to_owned()))
                        .and_then(|session| force_quit(session, &app));
                    if let Err(error) = quit {
                        tracing::warn!("cannot force quit {}: {error}", app.app_id);
                    }
                    self.refresh(Instant::now());
                }
                (Effect::ActivityMonitor, _) => {
                    let command = MONITOR_COMMAND.map(str::to_owned);
                    if let Err(error) = spawn_detached(ACTIVITY_MONITOR, &command, None, None) {
                        tracing::warn!("cannot open Activity Monitor: {error}");
                    }
                }
                (Effect::Close, _) => window.close(),
                (Effect::ForceQuit, None) => {}
            }
        }
    }
}

impl App for LowMemory {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.view.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let changed = match event {
            WindowEvent::CloseRequested => {
                window.close();
                false
            }
            WindowEvent::Input(input) => {
                let (handled, effects) = self.view.handle(&input);
                self.apply(window, effects);
                handled
            }
            WindowEvent::Accessibility(request) => self.view.ui().perform(&request),
            WindowEvent::AppearanceChanged(appearance) => {
                self.view.ui().set_appearance(appearance);
                true
            }
            _ => false,
        };
        if changed {
            window.request_redraw();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        Some(self.refreshed + REFRESH)
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.refresh(now);
        window.request_redraw();
    }

    fn animating(&self) -> bool {
        self.view.ui.is_animating()
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.view.ui.accessibility())
    }
}
