use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::L10N;
use unbound_status::devices::end_session;
use unbound_status::session::{COUNTDOWN, Ending, facts};
use unbound_toolkit::ui::{Axis, Button, Event, Key, Label, Stack, Ui, Widget};
use unbound_toolkit::{
    App, Image, Item, Palette, Point, Rect, Role, Scene, TextAlign, TextStyle, TextSystem, Window,
    WindowEvent,
};
use unbound_xdg::{Locale, applications, current_desktops, data_dirs, find_icon};
use zbus::blocking::Connection;

const ABOUT_SIZE: (u32, u32) = (280, 190);
const ABOUT_ICON: f32 = 64.0;
const ABOUT_PADDING: f32 = 28.0;
const ABOUT_GAP: f32 = 14.0;
const ABOUT_NAME_SIZE: f32 = 15.0;
const ABOUT_VERSION_SIZE: f32 = 11.0;

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Undecided,
    Cancel,
    Confirm,
}

struct Dialog {
    ui: Ui,
    choice: Rc<Cell<Choice>>,
    ending: Option<(Ending, Instant)>,
    next_second: Option<Instant>,
}

fn dialog_ui(lines: Vec<Box<dyn Widget>>, buttons: Vec<Box<dyn Widget>>, window: &Window) -> Ui {
    let mut rows = lines;
    if !buttons.is_empty() {
        rows.push(Box::new(Stack::new(Axis::Horizontal, 12.0, 0.0, buttons)));
    }
    Ui::new(
        Box::new(Stack::new(Axis::Vertical, 10.0, 20.0, rows)),
        unbound_toolkit::ui::Context::new(TextSystem::new(), window.appearance().palette()),
    )
}

fn confirmation(ending: Ending, left: Duration, choice: &Rc<Cell<Choice>>, window: &Window) -> Ui {
    let cancel = Rc::clone(choice);
    let confirm = Rc::clone(choice);
    dialog_ui(
        vec![
            Box::new(Label::new(&ending.question())),
            Box::new(Label::new(&ending.countdown(left))),
        ],
        vec![
            Box::new(Button::new(&L10N.tr("button-cancel"), move || {
                cancel.set(Choice::Cancel);
            })),
            Box::new(Button::new(&ending.button(), move || {
                confirm.set(Choice::Confirm);
            })),
        ],
        window,
    )
}

impl App for Dialog {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match &event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => self.choice.set(Choice::Cancel),
            WindowEvent::Input(Event::Key {
                key: Key::Enter, ..
            }) if self.ending.is_some() => self.choice.set(Choice::Confirm),
            WindowEvent::Input(input) => {
                if self.ui.handle(input) {
                    window.request_redraw();
                }
            }
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(request) {
                    window.request_redraw();
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(*appearance);
                window.request_redraw();
            }
            _ => {}
        }
        self.decide(window);
    }

    fn deadline(&self) -> Option<Instant> {
        self.next_second
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        let Some((ending, until)) = self.ending else {
            return;
        };
        let left = until.saturating_duration_since(now);
        if left.is_zero() {
            self.choice.set(Choice::Confirm);
        } else {
            self.ui = confirmation(
                ending,
                left + Duration::from_millis(999),
                &self.choice,
                window,
            );
            self.next_second = Some(now + Duration::from_secs(1));
            window.request_redraw();
        }
        self.decide(window);
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

impl Dialog {
    fn decide(&mut self, window: &mut Window) {
        match self.choice.get() {
            Choice::Undecided => {}
            Choice::Cancel => window.close(),
            Choice::Confirm => {
                if let Some((ending, _)) = self.ending {
                    let result =
                        Connection::system().and_then(|system| end_session(&system, ending));
                    report("end the session", result);
                }
                window.close();
            }
        }
    }
}

pub fn run_confirmation(ending: Ending) -> Result<(), Box<dyn std::error::Error>> {
    let mut window = Window::open(
        Role::toplevel(&ending.button(), "org.unbound.MenuBar"),
        (420, 130),
    )?;
    let choice = Rc::new(Cell::new(Choice::Undecided));
    let now = Instant::now();
    let ui = confirmation(ending, COUNTDOWN, &choice, &window);
    let mut dialog = Dialog {
        ui,
        choice,
        ending: Some((ending, now + COUNTDOWN)),
        next_second: Some(now + Duration::from_secs(1)),
    };
    unbound_toolkit::run(&mut window, &mut dialog)?;
    Ok(())
}

pub fn run_about() -> Result<(), Box<dyn std::error::Error>> {
    let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
    let model = std::fs::read("/proc/device-tree/model")
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let facts = facts(
        &read("/proc/sys/kernel/hostname"),
        &read("/etc/os-release"),
        model.as_deref(),
        &read("/proc/cpuinfo"),
        &read("/proc/meminfo"),
    );
    let mut window = Window::open(
        Role::toplevel(&L10N.tr("about-title"), "org.unbound.MenuBar"),
        (360, 210),
    )?;
    let row = |label: &str, value: &str| -> Box<dyn Widget> {
        Box::new(Label::new(&format!("{label}    {value}")))
    };
    let mut lines: Vec<Box<dyn Widget>> = vec![Box::new(Label::heading(&facts.name))];
    lines.push(row(&L10N.tr("about-system"), &facts.system));
    if !facts.chip.is_empty() {
        lines.push(row(&L10N.tr("about-chip"), &facts.chip));
    }
    lines.push(row(&L10N.tr("about-memory"), &facts.memory));
    lines.push(row(
        &L10N.tr("about-desktop"),
        &format!("Unbound {}", env!("CARGO_PKG_VERSION")),
    ));
    let ui = dialog_ui(lines, Vec::new(), &window);
    let mut dialog = Dialog {
        ui,
        choice: Rc::new(Cell::new(Choice::Undecided)),
        ending: None,
        next_second: None,
    };
    unbound_toolkit::run(&mut window, &mut dialog)?;
    Ok(())
}

struct AppAbout {
    name: String,
    version: String,
    icon: Option<Image>,
    palette: Palette,
    text: TextSystem,
}

impl App for AppAbout {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let width_points = width as f32;
        let centred = |text: &mut TextSystem, content: &str, style: TextStyle| {
            text.paragraph(content, style, Some(width_points), TextAlign::Center)
        };
        let name = centred(
            &mut self.text,
            &self.name,
            TextStyle::new(ABOUT_NAME_SIZE, self.palette.ink).weight(TextStyle::SEMIBOLD),
        );
        let version = centred(
            &mut self.text,
            &L10N.tr_args(
                "about-app-version",
                [("version", self.version.clone().into())],
            ),
            TextStyle::new(ABOUT_VERSION_SIZE, self.palette.ink2),
        );
        let mut items = Vec::new();
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: Rect::new(
                    ((width_points - ABOUT_ICON) / 2.0).round(),
                    ABOUT_PADDING,
                    ABOUT_ICON,
                    ABOUT_ICON,
                ),
                image: icon.clone(),
            });
        }
        let name_top = ABOUT_PADDING + ABOUT_ICON + ABOUT_GAP;
        let version_top = name_top + name.height() + 4.0;
        items.push(Item::Text {
            origin: Point::new(0.0, name_top),
            paragraph: name,
        });
        items.push(Item::Text {
            origin: Point::new(0.0, version_top),
            paragraph: version,
        });
        Scene {
            width,
            height,
            background: self.palette.sheet,
            items,
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => window.close(),
            WindowEvent::AppearanceChanged(appearance) => {
                self.palette = appearance.palette();
                window.request_redraw();
            }
            _ => {}
        }
    }
}

pub fn run_about_app(app_id: &str, version: &str) -> Result<(), Box<dyn std::error::Error>> {
    let directories = data_dirs();
    let entry = applications(
        &directories,
        Locale::from_env().as_ref(),
        &current_desktops(),
    )
    .into_iter()
    .find(|entry| entry.app_id() == app_id);
    let name = entry
        .as_ref()
        .map_or_else(|| app_id.to_owned(), |entry| entry.name.clone());
    let mut window = Window::open(
        Role::toplevel(
            &L10N.tr_args("about-app-title", [("app", name.clone().into())]),
            "org.unbound.MenuBar",
        ),
        ABOUT_SIZE,
    )?;
    let pixels = (f64::from(ABOUT_ICON) * window.scale().max(2.0)).round() as u32;
    let icon = entry
        .and_then(|entry| entry.icon)
        .and_then(|icon| find_icon(&icon, pixels, &[], &directories))
        .and_then(|path| Image::load(&path, pixels));
    let mut about = AppAbout {
        name,
        version: version.to_owned(),
        icon,
        palette: window.appearance().palette(),
        text: TextSystem::new(),
    };
    unbound_toolkit::run(&mut window, &mut about)?;
    Ok(())
}
