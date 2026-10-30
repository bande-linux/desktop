mod markup_app;

use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use accesskit::TreeUpdate;
use unbound_inbox::{Inbox, Outbox};
use unbound_screenshot::area::{AreaOutcome, AreaSelection, Region, crop};
use unbound_screenshot::color::ColorPicker;
use unbound_screenshot::engine::{Engine, EngineEvent, Shot, Source};
use unbound_screenshot::indicator::{Activity, Indicator};
use unbound_screenshot::options::{CaptureOptions, Destination, TIMERS};
use unbound_screenshot::palette::{Chosen, Mode as PaletteMode, Palette, PaletteAction};
use unbound_screenshot::picker::{Outcome, Picker};
use unbound_screenshot::record::{Container, Recording};
use unbound_screenshot::service::{self, Casting, SCREEN_ID, UiRequest};
use unbound_screenshot::shadow::with_shadow;
use unbound_screenshot::thumbnail::{Thumbnail, ThumbnailAction};
use unbound_screenshot::{L10N, app_name, free_path, png, recording_stem, save};
use unbound_toolkit::ui::{Alert, Context, FreeLayout, Menu, MenuItem, Size, Ui, Widget};
use unbound_toolkit::{
    App, Color, DragAction, DragIcon, Edges, Image, Item, KeyboardFocus, LayerKind, LayerOptions,
    MenuPlacement, Point, Rect, Role, Scene, Scheme, ScreenImage, Shape, TextAlign, TextStyle,
    TextSystem, Transfer, Window, WindowError, WindowEvent,
};
use unbound_xdg::{UserDir, file_uri, spawn_detached, user_dir};

const MARKUP_FLAG: &str = "--markup";
const PALETTE_BOTTOM: f32 = 96.0;
const COUNTDOWN_SIZE: f32 = 132.0;
const TICK: Duration = Duration::from_millis(16);

enum Purpose {
    Save,
    Portal(Sender<Option<PathBuf>>),
}

#[derive(Debug, Clone, PartialEq)]
enum Target {
    Screen,
    Window(String),
    Area(Region),
    RecordScreen,
    RecordArea(Region),
}

enum Mode {
    Idle,
    Area {
        selection: AreaSelection,
        purpose: Purpose,
        record: bool,
    },
    Palette(Chosen),
    Countdown(Instant, Target),
    Picking(Outcome, Sender<Option<Vec<String>>>),
    Asking {
        allowed: Rc<Cell<Option<bool>>>,
        interactive: bool,
        reply: Sender<Option<PathBuf>>,
    },
    Color(ColorPicker, Sender<Option<(f64, f64, f64)>>),
    Notice(Rc<Cell<bool>>),
}

struct Recorder {
    recording: Recording,
    target: Target,
    _indicator: Option<Indicator>,
}

struct Screenshots {
    home: PathBuf,
    config_home: PathBuf,
    options_path: PathBuf,
    options: CaptureOptions,
    palette_mode: PaletteMode,
    text: TextSystem,
    ui: Ui,
    engine: Option<Engine>,
    casting: Option<Arc<Casting>>,
    events: Inbox<EngineEvent>,
    requests: Inbox<UiRequest>,
    outbox: Outbox<UiRequest>,
    mode: Mode,
    thumbnail: Option<Thumbnail>,
    recorder: Option<Recorder>,
    container: Option<Container>,
}

fn as_screen_image(shot: &Shot) -> ScreenImage {
    ScreenImage {
        width: shot.width,
        height: shot.height,
        rgba: shot.rgba(),
    }
}

fn scaled(region: Region, scale: f64) -> Region {
    let at = |value: u32| (f64::from(value) * scale).round() as u32;
    let right = at(region.x + region.width);
    let bottom = at(region.y + region.height);
    Region {
        x: at(region.x),
        y: at(region.y),
        width: right - at(region.x),
        height: bottom - at(region.y),
    }
}

fn scrim(scheme: Scheme) -> Color {
    match scheme {
        Scheme::Light => Color::hex(0x000000).with_alpha(46),
        Scheme::Dark => Color::hex(0x000000).with_alpha(102),
    }
}

fn new_ui(window: &Window) -> Ui {
    let appearance = window.appearance();
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.context().keyboard_navigation = true;
    ui.context().reduce_motion = appearance.reduce_motion;
    ui.set_appearance(appearance);
    ui
}

fn thumbnail_image(image: &ScreenImage) -> Option<Image> {
    let scale = (480.0 / image.width.max(image.height).max(1) as f32).min(1.0);
    let width = ((image.width as f32 * scale).round() as u32).max(1);
    let height = ((image.height as f32 * scale).round() as u32).max(1);
    let source = image::RgbaImage::from_raw(image.width, image.height, image.rgba.clone())?;
    let small = image::imageops::resize(
        &source,
        width,
        height,
        image::imageops::FilterType::Triangle,
    );
    Image::from_rgba(width, height, small.into_raw())
}

impl Screenshots {
    fn user_dir(&self, dir: UserDir) -> PathBuf {
        let user_dirs = std::fs::read_to_string(self.config_home.join("user-dirs.dirs")).ok();
        user_dir(&self.home, user_dirs.as_deref(), dir)
    }

    fn portal_dir(&self) -> PathBuf {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".cache"))
            .join("unbound-screenshot")
    }

    fn destination_dir(&self) -> PathBuf {
        match self.options.destination {
            Destination::Pictures => self.user_dir(UserDir::Pictures),
            Destination::Desktop | Destination::Clipboard => self.user_dir(UserDir::Desktop),
        }
    }

    fn hide_overlay(&mut self, window: &mut Window) {
        if let Err(error) = window.hide() {
            tracing::warn!("cannot hide the overlay: {error}");
        }
    }

    fn arrange(&mut self, window: &mut Window) {
        let (width, height) = window.size();
        let (shown, region, keyboard) = match &self.mode {
            Mode::Idle => match &self.thumbnail {
                Some(thumbnail) => (
                    true,
                    Some(vec![thumbnail.rest((width, height))]),
                    KeyboardFocus::None,
                ),
                None => (false, None, KeyboardFocus::None),
            },
            Mode::Countdown(..) => (true, Some(Vec::new()), KeyboardFocus::None),
            _ => (true, None, KeyboardFocus::Exclusive),
        };
        if !shown {
            self.hide_overlay(window);
            return;
        }
        window.set_input_region(region.as_deref());
        if let Err(error) = window.set_keyboard_focus(keyboard) {
            tracing::warn!("cannot change the overlay's keyboard focus: {error}");
        }
        if let Err(error) = window.show() {
            tracing::warn!("cannot show the overlay: {error}");
        }
        window.request_redraw();
    }

    fn enter(&mut self, window: &mut Window, mode: Mode) {
        self.cancel(window);
        self.thumbnail = None;
        self.mode = mode;
        self.arrange(window);
    }

    fn cancel(&mut self, window: &mut Window) {
        match std::mem::replace(&mut self.mode, Mode::Idle) {
            Mode::Area {
                purpose: Purpose::Portal(reply),
                ..
            }
            | Mode::Asking { reply, .. } => {
                let _ = reply.send(None);
            }
            Mode::Picking(_, reply) => {
                let _ = reply.send(None);
            }
            Mode::Color(_, reply) => {
                let _ = reply.send(None);
            }
            Mode::Idle
            | Mode::Area { .. }
            | Mode::Palette(_)
            | Mode::Countdown(..)
            | Mode::Notice(_) => {}
        }
        self.ui = new_ui(window);
        self.arrange(window);
    }

    fn present(&mut self, window: &mut Window, dialog: impl Widget + 'static) {
        self.ui = new_ui(window);
        self.ui.present_dialog(dialog);
    }

    fn windows(&self) -> Vec<(String, unbound_screenshot::engine::Frame)> {
        self.engine
            .as_ref()
            .map(Engine::windows)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|window| Some((window.identifier, window.frame?)))
            .collect()
    }

    fn shoot(&mut self, window: &mut Window, target: &Target) -> Option<ScreenImage> {
        self.hide_overlay(window);
        let engine = self.engine.as_ref()?;
        let pointer = self.options.show_pointer;
        match target {
            Target::Window(identifier) => {
                let shot = engine.shoot(Source::Window(identifier.clone()), pointer)?;
                Some(with_shadow(&as_screen_image(&shot), window.scale()))
            }
            Target::Area(region) => {
                let image = as_screen_image(&engine.shoot(Source::Screen, pointer)?);
                crop(&image, scaled(*region, window.scale()))
            }
            _ => Some(as_screen_image(&engine.shoot(Source::Screen, pointer)?)),
        }
    }

    fn act(&mut self, window: &mut Window, target: Target) {
        match self.options.timer {
            0 => self.take(window, target),
            seconds => self.enter(
                window,
                Mode::Countdown(
                    Instant::now() + Duration::from_secs(u64::from(seconds)),
                    target,
                ),
            ),
        }
    }

    fn take(&mut self, window: &mut Window, target: Target) {
        self.mode = Mode::Idle;
        self.thumbnail = None;
        match target {
            Target::RecordScreen => self.start_recording(window, Target::RecordScreen),
            Target::RecordArea(region) => {
                self.start_recording(window, Target::RecordArea(region));
            }
            target => {
                let Some(image) = self.shoot(window, &target) else {
                    self.arrange(window);
                    return;
                };
                self.deliver(window, image);
            }
        }
        self.arrange(window);
    }

    fn deliver(&mut self, window: &mut Window, image: ScreenImage) {
        if self.options.destination == Destination::Clipboard {
            match png(&image) {
                Ok(bytes) => window.copy_transfer(Transfer::uris(Vec::new()).with_png(bytes)),
                Err(error) => tracing::warn!("cannot copy the screenshot: {error}"),
            }
            return;
        }
        let dir = self.destination_dir();
        let taken = jiff::Zoned::now().datetime();
        match save(image.clone(), &dir, taken) {
            Ok(path) => {
                tracing::info!("saved {}", path.display());
                self.show_thumbnail(window, &image, path, false);
            }
            Err(error) => tracing::warn!("cannot save to {}: {error}", dir.display()),
        }
    }

    fn show_thumbnail(
        &mut self,
        window: &mut Window,
        image: &ScreenImage,
        path: PathBuf,
        video: bool,
    ) {
        let Some(picture) = thumbnail_image(image) else {
            return;
        };
        let appearance = window.appearance();
        self.thumbnail = Some(Thumbnail::new(
            picture,
            path,
            video,
            appearance.palette().surface_elevated,
            appearance.reduce_motion,
            Instant::now(),
        ));
    }

    fn start_recording(&mut self, window: &mut Window, target: Target) {
        let (Some(casting), Some(container)) = (self.casting.clone(), self.container) else {
            let read = Rc::new(Cell::new(false));
            let dismissed = Rc::clone(&read);
            self.present(
                window,
                Alert::new(
                    &L10N.tr("cannot-record"),
                    &L10N.tr("no-recorder"),
                    move |_| {
                        dismissed.set(true);
                    },
                )
                .primary(&L10N.tr("ok"), "ok"),
            );
            self.mode = Mode::Notice(read);
            return;
        };
        self.hide_overlay(window);
        let Some((node, width, height)) = casting.open(SCREEN_ID, self.options.show_pointer, false)
        else {
            tracing::warn!("cannot record the screen");
            return;
        };
        let area = match &target {
            Target::RecordArea(region) => Some(scaled(*region, window.scale())),
            _ => None,
        };
        let taken = jiff::Zoned::now().datetime();
        let dir = self.destination_dir();
        if let Err(error) = std::fs::create_dir_all(&dir) {
            tracing::warn!("cannot create {}: {error}", dir.display());
        }
        let path = free_path(&dir, &recording_stem(taken), container.extension());
        match Recording::start(node, (width, height), area, container, path) {
            Ok(recording) => {
                let outbox = self.outbox.clone();
                let indicator = Indicator::show(
                    Activity::Recording,
                    &L10N.tr("recording-screen"),
                    move || {
                        outbox.send(UiRequest::StopRecording);
                    },
                )
                .inspect_err(|error| tracing::warn!("cannot show the recording indicator: {error}"))
                .ok();
                self.recorder = Some(Recorder {
                    recording,
                    target,
                    _indicator: indicator,
                });
            }
            Err(error) => {
                tracing::warn!("cannot start recording: {error}");
                casting.stop(&[node]);
            }
        }
    }

    fn stop_recording(&mut self, window: &mut Window) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        let still = self.shoot(
            window,
            &match recorder.target {
                Target::RecordArea(region) => Target::Area(region),
                _ => Target::Screen,
            },
        );
        let node = recorder.recording.node;
        let finished = recorder.recording.finish();
        if let Some(casting) = &self.casting {
            casting.stop(&[node]);
        }
        match finished {
            Ok(path) => {
                tracing::info!("recorded {}", path.display());
                if let Some(still) = still {
                    self.show_thumbnail(window, &still, path, true);
                }
            }
            Err(error) => tracing::warn!("the recording failed: {error}"),
        }
        self.arrange(window);
    }

    fn show_palette(&mut self, window: &mut Window) {
        if self.recorder.is_some() {
            self.stop_recording(window);
            return;
        }
        if matches!(self.mode, Mode::Palette(_)) {
            self.cancel(window);
            return;
        }
        self.cancel(window);
        let (width, height) = window.size();
        let mut layout = FreeLayout::new();
        let (mut palette, chosen) = Palette::new(self.palette_mode);
        let size = palette.measure(self.ui.context(), Size::new(width as f32, height as f32));
        layout.insert(
            palette,
            Rect::new(
                ((width as f32 - size.width) / 2.0).round(),
                height as f32 - PALETTE_BOTTOM - size.height,
                size.width,
                size.height,
            ),
        );
        let appearance = window.appearance();
        self.ui = Ui::new(
            Box::new(layout),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        self.ui.context().keyboard_navigation = true;
        self.ui.set_appearance(appearance);
        self.ui.handle(&unbound_toolkit::ui::Event::Key {
            key: unbound_toolkit::ui::Key::Tab,
            modifiers: Default::default(),
        });
        self.thumbnail = None;
        self.mode = Mode::Palette(chosen);
        self.arrange(window);
    }

    fn options_menu(&self) -> Menu {
        let options = self.options;
        let destination = |title: &str, action: &str, chosen: Destination| {
            MenuItem::new(&L10N.tr(title), action).checked(options.destination == chosen)
        };
        let mut items = vec![
            MenuItem::new(&L10N.tr("save-to"), "").enabled(false),
            destination("desktop", "save-desktop", Destination::Desktop),
            destination("pictures", "save-pictures", Destination::Pictures),
            destination("clipboard", "save-clipboard", Destination::Clipboard),
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("timer"), "").enabled(false),
        ];
        for seconds in TIMERS {
            let title = match seconds {
                0 => L10N.tr("timer-none"),
                _ => L10N.tr_args("timer-seconds", [("seconds", seconds.into())]),
            };
            items.push(
                MenuItem::new(&title, &format!("timer-{seconds}"))
                    .checked(options.timer == seconds),
            );
        }
        items.push(MenuItem::separator());
        items.push(
            MenuItem::new(&L10N.tr("show-pointer"), "show-pointer").checked(options.show_pointer),
        );
        Menu::new(items)
    }

    fn choose_option(&mut self, action: &str) {
        let options = &mut self.options;
        match action {
            "save-desktop" => options.destination = Destination::Desktop,
            "save-pictures" => options.destination = Destination::Pictures,
            "save-clipboard" => options.destination = Destination::Clipboard,
            "show-pointer" => options.show_pointer = !options.show_pointer,
            _ => match action
                .strip_prefix("timer-")
                .and_then(|seconds| seconds.parse().ok())
            {
                Some(seconds) => options.timer = seconds,
                None => return,
            },
        }
        if let Err(error) = self.options.store(&self.options_path) {
            tracing::warn!("cannot remember the capture options: {error}");
        }
    }

    fn settle_palette(&mut self, window: &mut Window) {
        let Mode::Palette(chosen) = &self.mode else {
            return;
        };
        let Some(action) = chosen.borrow_mut().take() else {
            return;
        };
        match action {
            PaletteAction::Close => self.cancel(window),
            PaletteAction::Options(anchor) => {
                if let Err(error) =
                    window.show_menu(self.options_menu(), MenuPlacement::Below(anchor))
                {
                    tracing::warn!("cannot show the options: {error}");
                }
            }
            PaletteAction::Capture(mode) => {
                self.palette_mode = mode;
                self.mode = Mode::Idle;
                self.ui = new_ui(window);
                match mode {
                    PaletteMode::Screen => self.act(window, Target::Screen),
                    PaletteMode::RecordScreen => self.act(window, Target::RecordScreen),
                    PaletteMode::Window | PaletteMode::Area | PaletteMode::RecordArea => {
                        let window_mode = mode == PaletteMode::Window;
                        let windows = match mode {
                            PaletteMode::RecordArea => Vec::new(),
                            _ => self.windows(),
                        };
                        self.enter(
                            window,
                            Mode::Area {
                                selection: AreaSelection::new(windows, window_mode),
                                purpose: Purpose::Save,
                                record: mode == PaletteMode::RecordArea,
                            },
                        );
                    }
                }
            }
        }
    }

    fn request(&mut self, window: &mut Window, request: UiRequest) {
        match request {
            UiRequest::Cancel => self.cancel(window),
            UiRequest::StopRecording => self.stop_recording(window),
            UiRequest::Pick {
                app_id,
                multiple,
                choices,
                reply,
            } => {
                self.cancel(window);
                let (picker, outcome) =
                    Picker::new(app_name(&app_id).as_deref(), choices, multiple);
                self.present(window, picker);
                self.enter_keeping_ui(window, Mode::Picking(outcome, reply));
            }
            UiRequest::Shot {
                app_id,
                interactive,
                ask,
                reply,
            } => {
                self.cancel(window);
                if ask {
                    let allowed = Rc::new(Cell::new(None));
                    let choice = Rc::clone(&allowed);
                    let name = app_name(&app_id).unwrap_or_else(|| L10N.tr("an-app"));
                    self.present(
                        window,
                        Alert::new(
                            &L10N.tr_args("allow-screenshot", [("app", name.into())]),
                            &L10N.tr("allow-screenshot-detail"),
                            move |action| choice.set(Some(action == "allow")),
                        )
                        .cancel(&L10N.tr("deny"), "deny")
                        .primary(&L10N.tr("allow"), "allow"),
                    );
                    self.enter_keeping_ui(
                        window,
                        Mode::Asking {
                            allowed,
                            interactive,
                            reply,
                        },
                    );
                } else {
                    self.take_for_portal(window, interactive, reply);
                }
            }
            UiRequest::PickColor { reply } => {
                self.cancel(window);
                let Some(shot) = self
                    .engine
                    .as_ref()
                    .and_then(|engine| engine.shoot(Source::Screen, false))
                else {
                    let _ = reply.send(None);
                    return;
                };
                self.enter(
                    window,
                    Mode::Color(ColorPicker::new(shot, window.scale()), reply),
                );
            }
        }
    }

    fn enter_keeping_ui(&mut self, window: &mut Window, mode: Mode) {
        self.thumbnail = None;
        self.mode = mode;
        self.arrange(window);
    }

    fn take_for_portal(
        &mut self,
        window: &mut Window,
        interactive: bool,
        reply: Sender<Option<PathBuf>>,
    ) {
        if interactive {
            let windows = self.windows();
            self.enter(
                window,
                Mode::Area {
                    selection: AreaSelection::new(windows, false),
                    purpose: Purpose::Portal(reply),
                    record: false,
                },
            );
            return;
        }
        self.portal_shot(window, &Target::Screen, reply);
    }

    fn portal_shot(
        &mut self,
        window: &mut Window,
        target: &Target,
        reply: Sender<Option<PathBuf>>,
    ) {
        let path = self.shoot(window, target).and_then(|image| {
            let dir = self.portal_dir();
            save(image, &dir, jiff::Zoned::now().datetime())
                .inspect_err(|error| tracing::warn!("cannot save to {}: {error}", dir.display()))
                .ok()
        });
        let _ = reply.send(path);
        self.mode = Mode::Idle;
        self.arrange(window);
    }

    fn chosen_area(&mut self, window: &mut Window, target: Target) {
        let Mode::Area {
            purpose, record, ..
        } = std::mem::replace(&mut self.mode, Mode::Idle)
        else {
            return;
        };
        match (purpose, target) {
            (Purpose::Portal(reply), target) => self.portal_shot(window, &target, reply),
            (Purpose::Save, Target::Area(region)) if record => {
                self.act(window, Target::RecordArea(region));
            }
            (Purpose::Save, target) => self.act(window, target),
        }
    }

    fn settle_dialogs(&mut self, window: &mut Window) {
        if self.ui.is_animating() {
            return;
        }
        match std::mem::replace(&mut self.mode, Mode::Idle) {
            Mode::Picking(outcome, reply) => {
                let chosen = outcome.borrow_mut().take();
                match chosen {
                    Some(chosen) => {
                        let _ = reply.send(chosen);
                        self.ui = new_ui(window);
                        self.arrange(window);
                    }
                    None => self.mode = Mode::Picking(outcome, reply),
                }
            }
            Mode::Asking {
                allowed,
                interactive,
                reply,
            } => match allowed.get() {
                Some(true) => {
                    self.ui = new_ui(window);
                    self.take_for_portal(window, interactive, reply);
                }
                Some(false) => {
                    let _ = reply.send(None);
                    self.ui = new_ui(window);
                    self.arrange(window);
                }
                None => {
                    self.mode = Mode::Asking {
                        allowed,
                        interactive,
                        reply,
                    }
                }
            },
            Mode::Notice(read) => match read.get() {
                true => {
                    self.ui = new_ui(window);
                    self.arrange(window);
                }
                false => self.mode = Mode::Notice(read),
            },
            other => self.mode = other,
        }
    }

    fn open_markup(&self, path: &Path) {
        let Ok(program) = std::env::current_exe() else {
            return;
        };
        let command = [
            program.to_string_lossy().into_owned(),
            MARKUP_FLAG.to_owned(),
            path.to_string_lossy().into_owned(),
        ];
        if let Err(error) = spawn_detached(&command, None, None) {
            tracing::warn!("cannot open markup: {error}");
        }
    }

    fn thumbnail_event(&mut self, window: &mut Window, input: &unbound_toolkit::ui::Event) {
        let screen = window.size();
        let now = Instant::now();
        let Some(thumbnail) = self.thumbnail.as_mut() else {
            return;
        };
        match thumbnail.handle(screen, input, now) {
            ThumbnailAction::Open(path) => {
                self.thumbnail = None;
                self.open_markup(&path);
                self.arrange(window);
            }
            ThumbnailAction::Drag(path) => {
                let icon = DragIcon {
                    scene: thumbnail.icon(),
                    hotspot: Point::new(24.0, 24.0),
                };
                let png = std::fs::read(&path)
                    .ok()
                    .filter(|_| path.extension().is_some_and(|extension| extension == "png"));
                let transfer = Transfer::uris(vec![file_uri(&path)]);
                let transfer = match png {
                    Some(bytes) => transfer.with_png(bytes),
                    None => transfer,
                };
                if let Err(error) = window.start_drag(transfer, &[DragAction::Copy], Some(icon)) {
                    tracing::warn!("cannot drag the screenshot: {error}");
                }
                thumbnail.dismiss(now);
                window.request_redraw();
            }
            ThumbnailAction::Gone => {
                self.thumbnail = None;
                self.arrange(window);
            }
            ThumbnailAction::None => window.request_redraw(),
        }
    }

    fn countdown_scene(&mut self, width: u32, height: u32, until: Instant) -> Scene {
        let left = until.saturating_duration_since(Instant::now());
        let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
        let frame = Rect::new(
            (width as f32 - COUNTDOWN_SIZE) / 2.0,
            (height as f32 - COUNTDOWN_SIZE) / 2.0,
            COUNTDOWN_SIZE,
            COUNTDOWN_SIZE,
        );
        let palette = self.ui.context().palette;
        let style = TextStyle::new(64.0, palette.label_primary).weight(TextStyle::SEMIBOLD);
        let paragraph = self
            .text
            .paragraph(&seconds.to_string(), style, None, TextAlign::Start);
        let origin = Point::new(
            frame.x + (frame.width - paragraph.width()) / 2.0,
            frame.y + (frame.height - paragraph.height()) / 2.0,
        );
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items: vec![
                Item::Fill {
                    shape: Shape::rounded(frame, 14.0),
                    paint: palette.surface_elevated.into(),
                },
                Item::Text { origin, paragraph },
            ],
        }
    }
}

impl App for Screenshots {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        let empty = Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items: Vec::new(),
        };
        match &self.mode {
            Mode::Area { selection, .. } => selection.scene(width, height, &mut self.text),
            Mode::Color(picker, _) => picker.scene(width, height, &mut self.text),
            Mode::Countdown(until, _) => {
                let until = *until;
                self.countdown_scene(width, height, until)
            }
            Mode::Picking(..) | Mode::Asking { .. } | Mode::Palette(_) | Mode::Notice(_) => {
                self.ui.layout(Size::new(width as f32, height as f32));
                let mut scene = self.ui.scene(width, height);
                scene.background = Color::TRANSPARENT;
                if matches!(self.mode, Mode::Palette(_)) {
                    scene.items.insert(
                        0,
                        Item::Fill {
                            shape: Shape::rect(Rect::new(0.0, 0.0, width as f32, height as f32)),
                            paint: scrim(window.appearance().scheme).into(),
                        },
                    );
                }
                scene
            }
            Mode::Idle => match &self.thumbnail {
                Some(thumbnail) => thumbnail.scene((width, height), Instant::now()),
                None => empty,
            },
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::CaptureScreen if self.recorder.is_none() => {
                if matches!(self.mode, Mode::Idle) {
                    self.take(window, Target::Screen);
                }
            }
            WindowEvent::CaptureArea if self.recorder.is_none() => {
                let windows = self.windows();
                self.enter(
                    window,
                    Mode::Area {
                        selection: AreaSelection::new(windows, false),
                        purpose: Purpose::Save,
                        record: false,
                    },
                );
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                window.request_redraw();
            }
            WindowEvent::MenuAction(action) => {
                self.choose_option(&action);
                window.request_redraw();
            }
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(&request) {
                    window.request_redraw();
                }
                self.settle_palette(window);
                self.settle_dialogs(window);
            }
            WindowEvent::Input(input) => match &mut self.mode {
                Mode::Area { selection, .. } => match selection.handle(&input) {
                    AreaOutcome::Selecting => window.request_redraw(),
                    AreaOutcome::Cancelled => self.cancel(window),
                    AreaOutcome::Chosen(region) => self.chosen_area(window, Target::Area(region)),
                    AreaOutcome::ChosenWindow(identifier) => {
                        self.chosen_area(window, Target::Window(identifier));
                    }
                },
                Mode::Color(picker, _) => {
                    if let Some(picked) = picker.handle(&input) {
                        if let Mode::Color(_, reply) = std::mem::replace(&mut self.mode, Mode::Idle)
                        {
                            let _ = reply.send(picked);
                        }
                        self.arrange(window);
                    } else {
                        window.request_redraw();
                    }
                }
                Mode::Picking(..) | Mode::Asking { .. } | Mode::Palette(_) | Mode::Notice(_) => {
                    if self.ui.handle(&input) {
                        window.request_redraw();
                    }
                    self.settle_palette(window);
                    self.settle_dialogs(window);
                }
                Mode::Idle => self.thumbnail_event(window, &input),
                Mode::Countdown(..) => {}
            },
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.events.fd(), self.requests.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        match index {
            0 => {
                for event in self.events.drain() {
                    match event {
                        EngineEvent::StreamEnded(stream) => {
                            if let Some(casting) = &self.casting {
                                casting.stream_ended(stream);
                            }
                        }
                        EngineEvent::ShowOptions => self.show_palette(window),
                    }
                }
            }
            _ => {
                for request in self.requests.drain() {
                    self.request(window, request);
                }
            }
        }
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        self.settle_dialogs(window);
        if let Mode::Countdown(until, _) = &self.mode {
            if *until <= now {
                if let Mode::Countdown(_, target) = std::mem::replace(&mut self.mode, Mode::Idle) {
                    self.take(window, target);
                }
            } else {
                window.request_redraw();
            }
        }
        if matches!(self.mode, Mode::Idle)
            && let Some(thumbnail) = self.thumbnail.as_mut()
        {
            if thumbnail.tick(now) == ThumbnailAction::Gone {
                self.thumbnail = None;
                self.arrange(window);
            } else {
                window.request_redraw();
            }
        }
    }

    fn deadline(&self) -> Option<Instant> {
        let now = Instant::now();
        let animating_dialog = matches!(
            self.mode,
            Mode::Picking(..) | Mode::Asking { .. } | Mode::Palette(_) | Mode::Notice(_)
        ) && self.ui.is_animating();
        let countdown = match &self.mode {
            Mode::Countdown(until, _) => Some((*until).min(now + Duration::from_millis(250))),
            _ => None,
        };
        let thumbnail =
            self.thumbnail
                .as_ref()
                .and_then(|thumbnail| match thumbnail.is_animating(now) {
                    true => Some(now + TICK),
                    false => thumbnail.deadline(),
                });
        [animating_dialog.then_some(now + TICK), countdown, thumbnail]
            .into_iter()
            .flatten()
            .min()
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
            || self
                .thumbnail
                .as_ref()
                .is_some_and(|thumbnail| thumbnail.is_animating(Instant::now()))
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        match self.mode {
            Mode::Picking(..) | Mode::Asking { .. } | Mode::Palette(_) | Mode::Notice(_) => {
                Some(self.ui.accessibility())
            }
            _ => None,
        }
    }
}

fn run() -> Result<(), WindowError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| WindowError::Io(std::io::Error::other("HOME is not set")))?;
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let options_path = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("unbound-screenshot/options");
    let mut window = Window::open(
        Role::Layer(LayerOptions {
            layer: LayerKind::Overlay,
            anchor: Edges::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardFocus::Exclusive,
            namespace: "screenshot".to_owned(),
        }),
        (0, 0),
    )?;
    window.hide()?;
    let events = Inbox::new()?;
    let requests = Inbox::new()?;
    let engine = Engine::start(window.connection(), events.outbox())
        .inspect_err(|error| tracing::warn!("cannot start capturing: {error}"))
        .ok();
    let casting = engine.clone().map(Casting::new);
    let outbox = requests.outbox();
    let _service = casting.as_ref().and_then(|casting| {
        service::serve(casting, requests.outbox())
            .inspect_err(|error| tracing::warn!("cannot offer capture to the portal: {error}"))
            .ok()
    });
    let mut screenshots = Screenshots {
        home,
        config_home,
        options: CaptureOptions::load(&options_path),
        options_path,
        palette_mode: PaletteMode::Area,
        text: TextSystem::new(),
        ui: new_ui(&window),
        engine,
        casting,
        events,
        requests,
        outbox,
        mode: Mode::Idle,
        thumbnail: None,
        recorder: None,
        container: Container::available(),
    };
    unbound_toolkit::run(&mut window, &mut screenshots)
}

fn main() -> ExitCode {
    unbound_log::init("unbound-screenshot");
    let arguments: Vec<String> = std::env::args().collect();
    let outcome = match arguments.as_slice() {
        [_, flag, path] if flag == MARKUP_FLAG => markup_app::run(Path::new(path)),
        _ => run(),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
