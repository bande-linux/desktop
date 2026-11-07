use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use crate::L10N;
use crate::status_client::report;
use bande_config::{BUS_NAME, Config, OBJECT_PATH, config_path};
use bande_inbox::Inbox;
use bande_status::devices::end_session;
use bande_status::session::{
    Effect, Ending, Facts, Refusal, SessionEnding, Stage, facts, quitting_apps, try_again,
};
use bande_toolkit::ui::{Alert, CheckState, Checkbox, Event, Form, FormLine, Key, Label, Size, Ui};
use bande_toolkit::{
    App, Appearance, Image, Item, Palette, Point, Rect, Role, Scene, Sizing, TextAlign, TextStyle,
    TextSystem, Window, WindowEvent,
};
use bande_xdg::{
    Locale, applications, crashed_apps_file, current_desktops, data_dirs, find_app, find_icon,
    forget_saved_windows, listed_apps, reopen_file, reopen_listed_apps, take_apps_to_reopen,
};

pub const DIALOGS: &str = "org.bande.MenuBar";
const ABOUT_SIZE: (u32, u32) = (280, 190);
const ABOUT_ICON: f32 = 64.0;
const ABOUT_PADDING: f32 = 28.0;
const ABOUT_GAP: f32 = 14.0;
const ABOUT_NAME_SIZE: f32 = 15.0;
const ABOUT_VERSION_SIZE: f32 = 11.0;

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

const CANCEL: &str = "cancel";
const CONFIRM: &str = "confirm";

fn confirmation_ui(
    session: &SessionEnding,
    choice: &Rc<Cell<Choice>>,
    reopen: &Rc<Cell<bool>>,
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
    let cancel = L10N.tr("button-cancel");
    let alert = match session.stage() {
        Stage::Asking { .. } => Alert::new(
            &ending.question(),
            &ending.countdown(session.left(now) + Duration::from_millis(999)),
            on_choice,
        )
        .accessory({
            let reopen = Rc::clone(reopen);
            let state = match reopen.get() {
                true => CheckState::Checked,
                false => CheckState::Unchecked,
            };
            Checkbox::new(&L10N.tr("reopen-windows"), state, move |on| reopen.set(on))
        })
        .primary(&ending.button(), CONFIRM)
        .cancel(&cancel, CANCEL),
        Stage::Quitting { .. } => {
            Alert::new(&quitting_apps(), "", on_choice).cancel(&cancel, CANCEL)
        }
        Stage::Blocked { app_id } => Alert::new(&ending.blocked(&app_name(app_id)), "", on_choice)
            .destructive(&ending.anyway(), CONFIRM)
            .cancel(&cancel, CANCEL),
        Stage::Ending => Alert::new(&ending.progress(), "", on_choice),
        Stage::Refused(refusal @ Refusal::Inhibited(_)) => {
            Alert::new(&ending.inhibited(), &refusal.explanation(), on_choice)
                .destructive(&ending.anyway(), CONFIRM)
                .cancel(&cancel, CANCEL)
        }
        Stage::Refused(refusal @ Refusal::Failed(_)) => {
            Alert::new(&ending.failed(), &refusal.explanation(), on_choice)
                .primary(&try_again(), CONFIRM)
                .cancel(&cancel, CANCEL)
        }
    };
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

struct Confirmation {
    session: SessionEnding,
    ui: Ui,
    size: (u32, u32),
    choice: Rc<Cell<Choice>>,
    reopen: Rc<Cell<bool>>,
    answers: Inbox<Refusal>,
}

impl Confirmation {
    fn rebuild(&mut self, window: &mut Window, now: Instant) {
        self.ui = confirmation_ui(
            &self.session,
            &self.choice,
            &self.reopen,
            window.appearance(),
            now,
        );
        let size = fitted(&mut self.ui);
        if size != self.size {
            self.size = size;
            window.resize_content(size, Sizing::Fixed);
        }
        window.request_redraw();
    }

    fn apply(&mut self, window: &mut Window, effect: Option<Effect>, now: Instant) {
        match effect {
            None => return,
            Some(Effect::QuitApps) => {
                report("let the apps ask first", window.set_keep_on_top(false));
                if let Err(error) = window.quit_all_apps() {
                    tracing::warn!("cannot ask the apps to quit: {error}");
                    let effect = self.session.confirm(now);
                    return self.apply(window, effect, now);
                }
            }
            Some(Effect::Activate(app_id)) => {
                report("bring the app forward", window.activate_app(&app_id))
            }
            Some(Effect::End { insist }) => {
                let ending = self.session.ending;
                let answer = self.answers.outbox();
                thread::spawn(move || {
                    if let Err(refusal) = end_session(ending, insist) {
                        answer.send(refusal);
                    }
                });
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
                remember_reopen_choice(self.reopen.get());
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
            }) if *self.session.stage() != Stage::Ending => self.choice.set(Choice::Cancel),
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

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.answers.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        for refusal in self.answers.drain() {
            self.session.refused(refusal);
        }
        self.rebuild(window, Instant::now());
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        *self.session.stage() != Stage::Ending
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
    let session = SessionEnding::new(ending, now);
    let choice = Rc::new(Cell::new(Choice::Undecided));
    let reopen = Rc::new(Cell::new(reopens_windows()));
    let mut ui = confirmation_ui(&session, &choice, &reopen, Appearance::default(), now);
    let size = fitted(&mut ui);
    let mut window = Window::open(Role::dialog(&ending.button(), DIALOGS), size)?;
    report("keep the dialog in front", window.set_keep_on_top(true));
    let mut confirmation = Confirmation {
        ui,
        session,
        size,
        choice,
        reopen,
        answers: Inbox::new()?,
    };
    confirmation.rebuild(&mut window, now);
    bande_toolkit::run(&mut window, &mut confirmation)?;
    Ok(())
}

fn reopens_windows() -> bool {
    let config_home = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    config_path(config_home.as_deref(), home.as_deref())
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|source| Config::load(&source).ok())
        .is_none_or(|(config, _)| config.reopen_apps)
}

fn remember_reopen_choice(on: bool) {
    let remembered = bande_bus::session().and_then(|session| {
        session.call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Set",
            &(
                "org.bande.Config",
                "ReopenApps",
                zbus::zvariant::Value::from(on),
            ),
        )
    });
    if let Err(error) = remembered {
        tracing::warn!("cannot remember whether to reopen the windows: {error}");
    }
}

fn reopen_question_ui(
    apps: &[String],
    answer: &Rc<Cell<Option<bool>>>,
    appearance: Appearance,
) -> Ui {
    let names: Vec<String> = apps.iter().map(|app| app_name(app)).collect();
    let answer = Rc::clone(answer);
    let alert = Alert::new(
        &L10N.tr("reopen-question"),
        &L10N.tr_args("reopen-explanation", [("apps", names.join(", ").into())]),
        move |action: &str| answer.set(Some(action == CONFIRM)),
    )
    .primary(&L10N.tr("reopen-button"), CONFIRM)
    .cancel(&L10N.tr("do-not-reopen-button"), CANCEL);
    let mut ui = Ui::new(
        Box::new(alert),
        bande_toolkit::ui::Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
}

struct ReopenQuestion {
    ui: Ui,
    size: (u32, u32),
    apps: Vec<String>,
    answer: Rc<Cell<Option<bool>>>,
}

impl ReopenQuestion {
    fn answered(&mut self, window: &mut Window) {
        let Some(reopen) = self.answer.take() else {
            return;
        };
        match reopen {
            true => reopen_listed_apps(&self.apps, |app| {
                report("place the reopened windows", window.reopening_app(app));
            }),
            false => forget_saved_windows(&self.apps),
        }
        if let Some(file) = crashed_apps_file() {
            take_apps_to_reopen(&file);
        }
        window.close();
    }
}

impl App for ReopenQuestion {
    fn scene(&mut self, _window: &mut Window) -> Scene {
        alert_scene(&mut self.ui, self.size)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(Event::Key {
                key: Key::Escape, ..
            }) => self.answer.set(Some(false)),
            WindowEvent::Input(Event::Key {
                key: Key::Enter, ..
            }) => self.answer.set(Some(true)),
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
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                window.request_redraw();
            }
            _ => {}
        }
        self.answered(window);
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        false
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

pub fn run_reopen_question() -> Result<(), Box<dyn std::error::Error>> {
    let apps = crashed_apps_file()
        .map(|file| listed_apps(&file))
        .unwrap_or_default();
    if apps.is_empty() {
        return Ok(());
    }
    let answer = Rc::new(Cell::new(None));
    let mut ui = reopen_question_ui(&apps, &answer, Appearance::default());
    let size = fitted(&mut ui);
    let mut window = Window::open(Role::dialog(&L10N.tr("reopen-question"), DIALOGS), size)?;
    report("keep the dialog in front", window.set_keep_on_top(true));
    let mut question = ReopenQuestion {
        ui,
        size,
        apps,
        answer,
    };
    bande_toolkit::run(&mut window, &mut question)?;
    Ok(())
}

const ABOUT_WIDTH: f32 = 360.0;

struct About {
    ui: Ui,
    size: (u32, u32),
}

impl About {
    fn refit(&mut self) {
        let size = self.ui.measure(Size::new(ABOUT_WIDTH, f32::INFINITY));
        self.size = (ABOUT_WIDTH as u32, size.height.ceil() as u32);
    }
}

impl App for About {
    fn scene(&mut self, _window: &mut Window) -> Scene {
        self.ui.scene(self.size.0, self.size.1)
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
                self.refit();
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn accessibility(&self) -> Option<accesskit::TreeUpdate> {
        Some(self.ui.accessibility())
    }
}

fn about_ui(facts: &Facts, appearance: Appearance) -> Ui {
    let row =
        |label: &str, value: &str| FormLine::Row(label.to_owned(), Box::new(Label::new(value)));
    let mut lines = vec![FormLine::Wide(Box::new(Label::heading(&facts.name)))];
    lines.push(row(&L10N.tr("about-system"), &facts.system));
    if !facts.chip.is_empty() {
        lines.push(row(&L10N.tr("about-chip"), &facts.chip));
    }
    lines.push(row(&L10N.tr("about-memory"), &facts.memory));
    lines.push(row(
        &L10N.tr("about-desktop"),
        &format!("Bande {}", env!("CARGO_PKG_VERSION")),
    ));
    let mut ui = Ui::new(
        Box::new(Form::new(&L10N.tr("about-title"), lines)),
        bande_toolkit::ui::Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.set_appearance(appearance);
    ui
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
    let mut about = About {
        ui: about_ui(&facts, Appearance::default()),
        size: (0, 0),
    };
    about.refit();
    let mut window = Window::open(Role::utility(&L10N.tr("about-title"), DIALOGS), about.size)?;
    about.ui = about_ui(&facts, window.appearance());
    about.refit();
    bande_toolkit::run(&mut window, &mut about)?;
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
        Role::utility(
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
