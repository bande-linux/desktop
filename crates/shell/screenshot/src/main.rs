use std::cell::Cell;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;

use accesskit::TreeUpdate;
use unbound_inbox::Inbox;
use unbound_screenshot::area::{AreaOutcome, AreaSelection, Region, crop};
use unbound_screenshot::color::ColorPicker;
use unbound_screenshot::engine::{Engine, EngineEvent, Shot, Source};
use unbound_screenshot::picker::{Outcome, Picker};
use unbound_screenshot::service::{self, Casting, UiRequest};
use unbound_screenshot::{L10N, app_name, save};
use unbound_toolkit::ui::{Alert, Context, FreeLayout, Size, Ui};
use unbound_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Role, Scene, ScreenImage,
    TextSystem, Window, WindowError, WindowEvent,
};
use unbound_xdg::{UserDir, user_dir};

enum Purpose {
    Save,
    Portal(Sender<Option<PathBuf>>),
}

enum Mode {
    Idle,
    Area(AreaSelection, Purpose),
    Picking(Outcome, Sender<Option<Vec<String>>>),
    Asking {
        allowed: Rc<Cell<Option<bool>>>,
        interactive: bool,
        reply: Sender<Option<PathBuf>>,
    },
    Color(ColorPicker, Sender<Option<(f64, f64, f64)>>),
}

struct Screenshots {
    home: PathBuf,
    config_home: PathBuf,
    text: TextSystem,
    ui: Ui,
    engine: Option<Engine>,
    casting: Option<Arc<Casting>>,
    events: Inbox<EngineEvent>,
    requests: Inbox<UiRequest>,
    mode: Mode,
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

impl Screenshots {
    fn shoot(&self, window: &mut Window, region: Option<Region>) -> Option<ScreenImage> {
        if let Err(error) = window.hide() {
            tracing::warn!("cannot hide the overlay: {error}");
        }
        let Some(engine) = &self.engine else {
            tracing::warn!("cannot capture the screen without the capture engine");
            return None;
        };
        let image = as_screen_image(&engine.shoot(Source::Screen, false)?);
        match region {
            Some(region) => crop(&image, scaled(region, window.scale())).or_else(|| {
                tracing::warn!("the chosen area is not on the screen");
                None
            }),
            None => Some(image),
        }
    }

    fn save_to(&self, image: ScreenImage, dir: PathBuf) -> Option<PathBuf> {
        let taken = jiff::Zoned::now().datetime();
        match save(image, &dir, taken) {
            Ok(path) => {
                tracing::info!("saved {}", path.display());
                Some(path)
            }
            Err(error) => {
                tracing::warn!("cannot save to {}: {error}", dir.display());
                None
            }
        }
    }

    fn desktop_dir(&self) -> PathBuf {
        let user_dirs = std::fs::read_to_string(self.config_home.join("user-dirs.dirs")).ok();
        user_dir(&self.home, user_dirs.as_deref(), UserDir::Desktop)
    }

    fn portal_dir(&self) -> PathBuf {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.home.join(".cache"))
            .join("unbound-screenshot")
    }

    fn finish(&mut self, window: &mut Window, region: Option<Region>, purpose: Purpose) {
        let image = self.shoot(window, region);
        match purpose {
            Purpose::Save => {
                if let Some(image) = image {
                    self.save_to(image, self.desktop_dir());
                }
            }
            Purpose::Portal(reply) => {
                let path = image.and_then(|image| self.save_to(image, self.portal_dir()));
                let _ = reply.send(path);
            }
        }
    }

    fn show(&mut self, window: &mut Window, mode: Mode) {
        self.cancel(window);
        self.mode = mode;
        if let Err(error) = window.show() {
            tracing::warn!("cannot show the overlay: {error}");
        }
        window.request_redraw();
    }

    fn hide(&mut self, window: &mut Window) {
        self.mode = Mode::Idle;
        if let Err(error) = window.hide() {
            tracing::warn!("cannot hide the overlay: {error}");
        }
    }

    fn cancel(&mut self, window: &mut Window) {
        match std::mem::replace(&mut self.mode, Mode::Idle) {
            Mode::Idle => return,
            Mode::Area(_, Purpose::Portal(reply)) | Mode::Asking { reply, .. } => {
                let _ = reply.send(None);
            }
            Mode::Area(_, Purpose::Save) => {}
            Mode::Picking(_, reply) => {
                let _ = reply.send(None);
            }
            Mode::Color(_, reply) => {
                let _ = reply.send(None);
            }
        }
        self.ui = new_ui(window);
        self.hide(window);
    }

    fn present(&mut self, window: &mut Window, dialog: impl unbound_toolkit::ui::Widget + 'static) {
        self.ui = new_ui(window);
        self.ui.present_dialog(dialog);
        if let Err(error) = window.show() {
            tracing::warn!("cannot show the overlay: {error}");
        }
        window.request_redraw();
    }

    fn request(&mut self, window: &mut Window, request: UiRequest) {
        match request {
            UiRequest::Cancel => self.cancel(window),
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
                self.mode = Mode::Picking(outcome, reply);
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
                    self.mode = Mode::Asking {
                        allowed,
                        interactive,
                        reply,
                    };
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
                self.show(
                    window,
                    Mode::Color(ColorPicker::new(shot, window.scale()), reply),
                );
            }
        }
    }

    fn take_for_portal(
        &mut self,
        window: &mut Window,
        interactive: bool,
        reply: Sender<Option<PathBuf>>,
    ) {
        match interactive {
            true => self.show(
                window,
                Mode::Area(AreaSelection::default(), Purpose::Portal(reply)),
            ),
            false => self.finish(window, None, Purpose::Portal(reply)),
        }
    }

    fn settle_dialogs(&mut self, window: &mut Window) {
        if self.ui.is_animating() {
            return;
        }
        match std::mem::replace(&mut self.mode, Mode::Idle) {
            Mode::Picking(outcome, reply) => match outcome.borrow_mut().take() {
                Some(chosen) => {
                    let _ = reply.send(chosen);
                    self.hide(window);
                }
                None => self.mode = Mode::Picking(Rc::clone(&outcome), reply),
            },
            Mode::Asking {
                allowed,
                interactive,
                reply,
            } => match allowed.get() {
                Some(true) => {
                    self.hide(window);
                    self.take_for_portal(window, interactive, reply);
                }
                Some(false) => {
                    let _ = reply.send(None);
                    self.hide(window);
                }
                None => {
                    self.mode = Mode::Asking {
                        allowed,
                        interactive,
                        reply,
                    }
                }
            },
            other => self.mode = other,
        }
    }
}

fn new_ui(window: &Window) -> Ui {
    let appearance = window.appearance();
    let mut ui = Ui::new(
        Box::new(FreeLayout::new()),
        Context::new(TextSystem::new(), appearance.palette()),
    );
    ui.context().keyboard_navigation = true;
    ui.set_appearance(appearance);
    ui
}

impl App for Screenshots {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let (width, height) = window.size();
        match &self.mode {
            Mode::Area(selection, _) => selection.scene(width, height, &mut self.text),
            Mode::Color(picker, _) => picker.scene(width, height, &mut self.text),
            Mode::Picking(..) | Mode::Asking { .. } => {
                self.ui.layout(Size::new(width as f32, height as f32));
                let mut scene = self.ui.scene(width, height);
                scene.background = Color::TRANSPARENT;
                scene
            }
            Mode::Idle => AreaSelection::default().scene(width, height, &mut self.text),
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::CaptureScreen if matches!(self.mode, Mode::Idle) => {
                self.finish(window, None, Purpose::Save);
            }
            WindowEvent::CaptureArea if matches!(self.mode, Mode::Idle) => {
                self.show(window, Mode::Area(AreaSelection::default(), Purpose::Save));
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.ui.set_appearance(appearance);
                window.request_redraw();
            }
            WindowEvent::Accessibility(request) => {
                if self.ui.perform(&request) {
                    window.request_redraw();
                }
                self.settle_dialogs(window);
            }
            WindowEvent::Input(input) => match &mut self.mode {
                Mode::Area(selection, _) => match selection.handle(&input) {
                    AreaOutcome::Selecting => window.request_redraw(),
                    AreaOutcome::Cancelled => self.cancel(window),
                    AreaOutcome::Chosen(region) => {
                        if let Mode::Area(_, purpose) =
                            std::mem::replace(&mut self.mode, Mode::Idle)
                        {
                            self.finish(window, Some(region), purpose);
                        }
                    }
                },
                Mode::Color(picker, _) => match picker.handle(&input) {
                    None => window.request_redraw(),
                    Some(picked) => {
                        if let Mode::Color(_, reply) = std::mem::replace(&mut self.mode, Mode::Idle)
                        {
                            let _ = reply.send(picked);
                        }
                        self.hide(window);
                    }
                },
                Mode::Picking(..) | Mode::Asking { .. } => {
                    if self.ui.handle(&input) {
                        window.request_redraw();
                    }
                    self.settle_dialogs(window);
                }
                Mode::Idle => {}
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
                        EngineEvent::ShowOptions => {}
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

    fn tick(&mut self, window: &mut Window, _now: std::time::Instant) {
        self.settle_dialogs(window);
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        match self.mode {
            Mode::Picking(..) | Mode::Asking { .. } if self.ui.is_animating() => {
                Some(std::time::Instant::now() + std::time::Duration::from_millis(16))
            }
            _ => None,
        }
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        match self.mode {
            Mode::Picking(..) | Mode::Asking { .. } => Some(self.ui.accessibility()),
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
    let _service = casting.as_ref().and_then(|casting| {
        service::serve(casting, requests.outbox())
            .inspect_err(|error| tracing::warn!("cannot offer capture to the portal: {error}"))
            .ok()
    });
    let mut screenshots = Screenshots {
        home,
        config_home,
        text: TextSystem::new(),
        ui: new_ui(&window),
        engine,
        casting,
        events,
        requests,
        mode: Mode::Idle,
    };
    unbound_toolkit::run(&mut window, &mut screenshots)
}

fn main() -> ExitCode {
    unbound_log::init("unbound-screenshot");
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            ExitCode::FAILURE
        }
    }
}
