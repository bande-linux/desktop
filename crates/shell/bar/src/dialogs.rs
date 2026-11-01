use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::L10N;
use bande_status::devices::end_session;
use bande_status::session::{Effect, Ending, SessionEnding, Stage, facts, quitting_apps};
use bande_toolkit::ui::{Alert, Axis, Event, Key, Label, Size, Stack, Ui, Widget};
use bande_toolkit::{
    App, Appearance, Image, Item, Palette, Point, Rect, Role, Scene, TextAlign, TextStyle,
    TextSystem, Window, WindowEvent,
};
use bande_xdg::{
    Locale, applications, current_desktops, data_dirs, find_app, find_icon, reopen_file,
    take_apps_to_reopen,
};
use zbus::blocking::Connection;

pub const DIALOGS: &str = "org.bande.MenuBar";
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

fn app_name(app_id: &str) -> String {
    let entries = applications(
        &data_dirs(),
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    find_app(&entries, app_id).map_or_else(|| app_id.to_owned(), |entry| entry.name.clone())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Undecided,
    Cancel,
    Confirm,
}

fn dialog_ui(lines: Widgets, palette: Palette) -> Ui {
    Ui::new(
        Box::new(Stack::new(Axis::Vertical, 10.0, 20.0, lines)),
        bande_toolkit::ui::Context::new(TextSystem::new(), palette),
    )
}

type Widgets = Vec<Box<dyn Widget>>;

const CANCEL: &str = "cancel";
const CONFIRM: &str = "confirm";

fn confirmation_ui(
    session: &SessionEnding,
    choice: &Rc<Cell<Choice>>,
    appearance: Appearance,
    now: Instant,
) -> Ui {
    let ending = session.ending;
    let picked = Rc::clone(choice);
    let on_choice = move |action: &str| {
        picked.set(match action {
            CONFIRM => Choice::Confirm,
            _ => Choice::Cancel,
        })
    };
    let alert = match session.stage() {
        Stage::Asking { .. } => Alert::new(
            &ending.question(),
            &ending.countdown(session.left(now) + Duration::from_millis(999)),
            on_choice,
        )
        .primary(&ending.button(), CONFIRM),
        Stage::Quitting { .. } => Alert::new(&quitting_apps(), "", on_choice),
        Stage::Blocked { app_id } => Alert::new(&ending.blocked(&app_name(app_id)), "", on_choice)
            .destructive(&ending.anyway(), CONFIRM),
        Stage::Ending => Alert::new("", "", on_choice),
    }
    .cancel(&L10N.tr("button-cancel"), CANCEL);
    let mut ui = Ui::new(
        Box::new(alert),
        bande_toolkit::ui::Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}

pub(crate) fn alert_scene(ui: &mut Ui, (width, height): (u32, u32)) -> Scene {
    let mut scene = ui.scene(width, height);
    scene.background = ui.context().palette.overlay;
    scene
}

pub(crate) fn fitted(ui: &mut Ui) -> (u32, u32) {
    let size = ui.measure(Size::new(f32::INFINITY, f32::INFINITY));
    (size.width.ceil() as u32, size.height.ceil() as u32)
}

fn confirmation_size(ending: Ending, appearance: Appearance, now: Instant) -> (u32, u32) {
    let asking = SessionEnding::new(ending, now);
    let choice = Rc::new(Cell::new(Choice::Undecided));
    fitted(&mut confirmation_ui(&asking, &choice, appearance, now))
}

pub fn session_ending_scene(
    session: &SessionEnding,
    appearance: Appearance,
    now: Instant,
) -> Scene {
    let choice = Rc::new(Cell::new(Choice::Undecided));
    let size = confirmation_size(session.ending, appearance, now);
    alert_scene(
        &mut confirmation_ui(session, &choice, appearance, now),
        size,
    )
}

struct Confirmation {
    session: SessionEnding,
    ui: Ui,
    size: (u32, u32),
    choice: Rc<Cell<Choice>>,
}

impl Confirmation {
    fn rebuild(&mut self, window: &mut Window, now: Instant) {
        let appearance = window.appearance();
        self.size = confirmation_size(self.session.ending, appearance, now);
        self.ui = confirmation_ui(&self.session, &self.choice, appearance, now);
        window.request_redraw();
    }

    fn apply(&mut self, window: &mut Window, effect: Option<Effect>, now: Instant) {
        match effect {
            None => return,
            Some(Effect::QuitApps) => {
                if let Err(error) = window.quit_all_apps() {
                    tracing::warn!("cannot ask the apps to quit: {error}");
                    let effect = self.session.confirm(now);
                    return self.apply(window, effect, now);
                }
            }
            Some(Effect::Activate(app_id)) => {
                report("bring the app forward", window.activate_app(&app_id))
            }
            Some(Effect::End) => {
                let ending = self.session.ending;
                let result = Connection::system().and_then(|system| end_session(&system, ending));
                report("end the session", result);
                window.close();
                return;
            }
        }
        self.rebuild(window, now);
    }

    fn decide(&mut self, window: &mut Window) {
        let now = Instant::now();
        match self.choice.replace(Choice::Undecided) {
            Choice::Undecided => {}
            Choice::Cancel => {
                if let Some(file) = reopen_file() {
                    take_apps_to_reopen(&file);
                }
                window.close();
            }
            Choice::Confirm => {
                let effect = self.session.confirm(now);
                self.apply(window, effect, now);
            }
        }
    }
}

impl App for Confirmation {
    fn scene(&mut self, _window: &mut Window) -> Scene {
        alert_scene(&mut self.ui, self.size)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => self.choice.set(Choice::Cancel),
            WindowEvent::Input(Event::Key {
                key: Key::Enter, ..
            }) if matches!(self.session.stage(), Stage::Asking { .. }) => {
                self.choice.set(Choice::Confirm);
            }
            WindowEvent::Input(input) => {
                if self.ui.handle(&input) {
                    window.request_redraw();
                }
            }
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(&request) {
                    window.request_redraw();
                }
            }
            WindowEvent::AppearanceChanged(_) => self.rebuild(window, Instant::now()),
            WindowEvent::AppsRunning { app_id, count } => {
                let now = Instant::now();
                let effect = self.session.apps_running(app_id, count, now);
                self.apply(window, effect, now);
            }
            _ => {}
        }
        self.decide(window);
    }

    fn deadline(&self) -> Option<Instant> {
        self.session.deadline(Instant::now())
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        match self.session.tick(now) {
            None => self.rebuild(window, now),
            effect => self.apply(window, effect, now),
        }
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

pub fn run_confirmation(ending: Ending) -> Result<(), Box<dyn std::error::Error>> {
    let now = Instant::now();
    let size = confirmation_size(ending, Appearance::default(), now);
    let mut window = Window::open(Role::dialog(&ending.button(), DIALOGS), size)?;
    report("keep the dialog in front", window.set_keep_on_top(true));
    let session = SessionEnding::new(ending, now);
    let choice = Rc::new(Cell::new(Choice::Undecided));
    let mut confirmation = Confirmation {
        ui: confirmation_ui(&session, &choice, window.appearance(), now),
        session,
        size,
        choice,
    };
    confirmation.rebuild(&mut window, now);
    bande_toolkit::run(&mut window, &mut confirmation)?;
    Ok(())
}

struct About {
    ui: Ui,
}

impl App for About {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        self.ui.scene(width, height)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => window.close(),
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(&request) {
                    window.request_redraw();
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
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
    let mut window = Window::open(Role::toplevel(&L10N.tr("about-title"), DIALOGS), (360, 210))?;
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
        &format!("Bande {}", env!("CARGO_PKG_VERSION")),
    ));
    let ui = dialog_ui(lines, window.appearance().palette());
    bande_toolkit::run(&mut window, &mut About { ui })?;
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
    let entries = applications(
        &directories,
        Locale::from_env().as_ref(),
        &current_desktops(),
    );
    let entry = find_app(&entries, app_id).cloned();
    let name = entry
        .as_ref()
        .map_or_else(|| app_id.to_owned(), |entry| entry.name.clone());
    let mut window = Window::open(
        Role::toplevel(
            &L10N.tr_args("about-app-title", [("app", name.clone().into())]),
            DIALOGS,
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
    bande_toolkit::run(&mut window, &mut about)?;
    Ok(())
}
