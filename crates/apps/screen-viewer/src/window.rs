use std::cell::{Cell, RefCell};
use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::rc::Rc;
use std::time::Instant;

use accesskit::TreeUpdate;
use bande_app::View;
use bande_l10n::tr;
use bande_rfb::client::{Command, Credentials};
use bande_toolkit::ui::{
    Axis, Button, ClipboardRequest, Context, Label, PointerButton, PointerCapture, SecureField,
    Stack, SurfaceInput, SurfaceScaling, SurfaceView, TextField, Ui, Widget, WidgetId,
};
use bande_toolkit::{
    Appearance, ExternalSurface, Point, Rect, Scene, ShmLayout, TextSystem, Window, WindowEvent,
};

use crate::address::Target;
use crate::connection::{Answer, Commander, Connection, Event, Question};

const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
const SPACING: f32 = 12.0;
const PADDING: f32 = 24.0;
const FIELD_SIZE: f32 = 13.0;
const WHEEL_STEP: f32 = 15.0;
const SCROLL_UP: u8 = 1 << 3;
const SCROLL_DOWN: u8 = 1 << 4;
const SCROLL_LEFT: u8 = 1 << 5;
const SCROLL_RIGHT: u8 = 1 << 6;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nearby {
    pub name: String,
    pub target: Target,
}

enum Intent {
    Connect(Target),
    ConnectTyped,
    Answer(Answer),
    Password(String),
    Choose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Stage {
    Choosing(Option<String>),
    Connecting,
    Asking(Question),
    Viewing,
}

type Intents = Rc<RefCell<Vec<Intent>>>;

pub struct Pane {
    ui: Ui,
    content: WidgetId,
    intents: Intents,
    stage: Stage,
    connection: Option<Connection>,
    target: Option<Target>,
    typed: Rc<RefCell<String>>,
    user: Rc<RefCell<String>>,
    nearby: Vec<Nearby>,
    nearby_missing: bool,
    surface: Option<ExternalSurface>,
    memory: Option<File>,
    picture: Option<(u32, u32)>,
    view: Option<WidgetId>,
    scaling: SurfaceScaling,
    from_server: Option<String>,
    title: String,
    encrypted: bool,
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

fn button(intents: &Intents, title: &str, intent: impl Fn() -> Intent + 'static) -> Button {
    let intents = Rc::clone(intents);
    Button::new(title, move || intents.borrow_mut().push(intent()))
}

fn rfb_button(button: PointerButton) -> u8 {
    match button {
        PointerButton::Primary => 1,
        PointerButton::Middle => 1 << 1,
        PointerButton::Secondary => 1 << 2,
        PointerButton::Back | PointerButton::Forward => 0,
    }
}

fn pass_on(
    commander: &Commander,
    buttons: &Cell<u8>,
    at: &Cell<Point>,
    gathered: &Cell<Point>,
    input: SurfaceInput,
) {
    let pointer = |buttons: u8, at: Point| Command::Pointer {
        x: at.x as u16,
        y: at.y as u16,
        buttons,
    };
    match input {
        SurfaceInput::PointerMoved(point) => {
            at.set(point);
            commander.send(pointer(buttons.get(), point));
        }
        SurfaceInput::Button { button, pressed } => {
            let bit = rfb_button(button);
            buttons.set(match pressed {
                true => buttons.get() | bit,
                false => buttons.get() & !bit,
            });
            commander.send(pointer(buttons.get(), at.get()));
        }
        SurfaceInput::Scroll { delta, .. } => {
            let mut total = Point::new(gathered.get().x + delta.x, gathered.get().y + delta.y);
            let wheel = |value: &mut f32, negative: u8, positive: u8| {
                while value.abs() >= WHEEL_STEP {
                    let bit = match *value < 0.0 {
                        true => negative,
                        false => positive,
                    };
                    commander.send(pointer(buttons.get() | bit, at.get()));
                    commander.send(pointer(buttons.get(), at.get()));
                    *value -= WHEEL_STEP.copysign(*value);
                }
            };
            wheel(&mut total.y, SCROLL_UP, SCROLL_DOWN);
            wheel(&mut total.x, SCROLL_LEFT, SCROLL_RIGHT);
            gathered.set(total);
        }
        SurfaceInput::Key {
            keysym, pressed, ..
        } if keysym != 0 => commander.send(Command::Key {
            keysym,
            down: pressed,
        }),
        SurfaceInput::Key { .. }
        | SurfaceInput::PointerLeft
        | SurfaceInput::Motion { .. }
        | SurfaceInput::Captured(_) => {}
    }
}

impl Pane {
    pub fn new(appearance: Appearance, target: Option<Target>) -> Self {
        let content = Stack::new(Axis::Vertical, 0.0, 0.0, vec![boxed(Label::new(""))]);
        let id = content.id();
        let mut ui = Ui::new(
            boxed(content),
            Context::new(TextSystem::new(), appearance.palette()),
        );
        ui.set_appearance(appearance);
        let mut pane = Self {
            ui,
            content: id,
            intents: Rc::default(),
            stage: Stage::Choosing(None),
            connection: None,
            target: None,
            typed: Rc::default(),
            user: Rc::default(),
            nearby: Vec::new(),
            nearby_missing: false,
            surface: None,
            memory: None,
            picture: None,
            view: None,
            scaling: SurfaceScaling::Fit,
            from_server: None,
            title: tr!("Screen Sharing"),
            encrypted: false,
        };
        match target {
            Some(target) => pane.connect(target),
            None => pane.rebuild(),
        }
        pane
    }

    fn window_title(&self) -> String {
        match self.stage == Stage::Viewing && !self.encrypted {
            true => tr!("{computer} (not encrypted)", computer = self.title.clone()),
            false => self.title.clone(),
        }
    }

    pub fn is_viewing(&self) -> bool {
        self.stage == Stage::Viewing
    }

    pub fn set_nearby(&mut self, nearby: Vec<Nearby>, missing: bool) {
        if (self.nearby.as_slice(), self.nearby_missing) == (nearby.as_slice(), missing) {
            return;
        }
        self.nearby = nearby;
        self.nearby_missing = missing;
        if matches!(self.stage, Stage::Choosing(_)) {
            self.rebuild();
        }
    }

    pub fn set_scaling(&mut self, scaling: SurfaceScaling) {
        self.scaling = scaling;
        if let Some(view) = self
            .view
            .and_then(|view| self.ui.widget_as::<SurfaceView>(view))
        {
            view.set_scaling(scaling);
        }
    }

    pub fn scaling(&self) -> SurfaceScaling {
        self.scaling
    }

    fn connect(&mut self, target: Target) {
        self.title = target.host.clone();
        match Connection::start(target.clone()) {
            Ok(connection) => {
                self.connection = Some(connection);
                self.target = Some(target);
                self.stage = Stage::Connecting;
            }
            Err(error) => {
                tracing::warn!("cannot start a connection: {error}");
                self.stage = Stage::Choosing(Some(error.to_string()));
            }
        }
        self.rebuild();
    }

    fn set_content(&mut self, widget: Box<dyn Widget>) {
        if let Some(content) = self.ui.widget(self.content)
            && let Some(slot) = content.children_mut().first_mut()
        {
            *slot = widget;
        }
    }

    fn page(&self, widgets: Vec<Box<dyn Widget>>) -> Box<dyn Widget> {
        boxed(Stack::new(Axis::Vertical, SPACING, PADDING, widgets))
    }

    fn choosing(&self, problem: Option<&str>) -> Vec<Box<dyn Widget>> {
        let mut widgets = vec![boxed(Label::heading(&tr!("Connect to a Screen")))];
        if let Some(problem) = problem {
            widgets.push(boxed(Label::new(problem)));
        }
        let typed = Rc::clone(&self.typed);
        let text = typed.borrow().clone();
        widgets.push(boxed(TextField::new(
            &tr!("Address, such as vnc://computer.local"),
            &text,
            move |text| *typed.borrow_mut() = text.to_owned(),
        )));
        widgets.push(boxed(
            button(&self.intents, &tr!("Connect"), || Intent::ConnectTyped).primary(),
        ));
        widgets.push(boxed(Label::heading(&tr!("Nearby"))));
        if self.nearby.is_empty() {
            let note = match self.nearby_missing {
                true => tr!("Finding computers nearby needs Avahi, which isn’t installed."),
                false => tr!("No computer nearby shares its screen."),
            };
            widgets.push(boxed(Label::new(&note).secondary()));
        }
        for nearby in &self.nearby {
            let target = nearby.target.clone();
            widgets.push(boxed(button(&self.intents, &nearby.name, move || {
                Intent::Connect(target.clone())
            })));
        }
        widgets
    }

    fn asking(&self, question: &Question) -> Vec<Box<dyn Widget>> {
        let computer = self
            .target
            .as_ref()
            .map(|target| target.host.clone())
            .unwrap_or_default();
        let yes = |title: &str| button(&self.intents, title, || Intent::Answer(Answer::Yes));
        let no = button(&self.intents, &tr!("Cancel"), || Intent::Answer(Answer::No)).cancel();
        match question {
            Question::Trust {
                fingerprint,
                changed: false,
            } => vec![
                boxed(Label::heading(&tr!(
                    "Is this {computer}?",
                    computer = computer.clone()
                ))),
                boxed(Label::new(&tr!(
                    "This is the first connection to {computer}. Check that its certificate fingerprint, shown in Settings ▸ Sharing on that computer, is the same:",
                    computer = computer
                ))),
                boxed(Label::new(fingerprint)),
                boxed(yes(&tr!("Trust and Connect")).primary()),
                boxed(no),
            ],
            Question::Trust {
                fingerprint,
                changed: true,
            } => vec![
                boxed(Label::heading(&tr!(
                    "{computer} has a different certificate",
                    computer = computer.clone()
                ))),
                boxed(Label::new(&tr!(
                    "The certificate isn’t the one {computer} had before. It may have been set up again, or another computer may be pretending to be it. Connect only if you know why it changed. Its fingerprint now:",
                    computer = computer
                ))),
                boxed(Label::new(fingerprint)),
                boxed(no.primary()),
                boxed(yes(&tr!("Trust the New Certificate")).destructive()),
            ],
            Question::Unencrypted => vec![
                boxed(Label::heading(&tr!("This connection isn’t encrypted"))),
                boxed(Label::new(&tr!(
                    "{computer} offers no encryption. Others on the network could see its screen, what you type and its password. Connect only on a network you trust.",
                    computer = computer
                ))),
                boxed(no.primary()),
                boxed(yes(&tr!("Connect Without Encryption")).destructive()),
            ],
            Question::Password { user, refused } => {
                let mut widgets = vec![boxed(Label::heading(&tr!(
                    "Sign in to {computer}",
                    computer = computer
                )))];
                if let Some(refused) = refused {
                    widgets.push(boxed(Label::new(refused)));
                }
                if let Some(user) = user {
                    *self.user.borrow_mut() = user.clone();
                    let typed = Rc::clone(&self.user);
                    widgets.push(boxed(TextField::new(&tr!("Name"), user, move |text| {
                        *typed.borrow_mut() = text.to_owned();
                    })));
                }
                let intents = Rc::clone(&self.intents);
                widgets.push(boxed(
                    SecureField::new(&tr!("Password"), FIELD_SIZE, move |password| {
                        intents
                            .borrow_mut()
                            .push(Intent::Password(password.to_owned()));
                    })
                    .with_go_button(),
                ));
                widgets.push(boxed(no));
                widgets
            }
        }
    }

    fn viewing(&mut self) -> Box<dyn Widget> {
        let (Some(surface), Some((width, height)), Some(connection)) =
            (&self.surface, self.picture, &self.connection)
        else {
            return boxed(Label::new(&tr!("Waiting for the screen…")));
        };
        let commander = connection.commander();
        let (buttons, at, gathered) = (
            Cell::new(0),
            Cell::new(Point::new(0.0, 0.0)),
            Cell::new(Point::new(0.0, 0.0)),
        );
        let label = tr!("Screen of {computer}", computer = self.title.clone());
        let mut view = SurfaceView::new(surface.id(), &label, move |input| {
            pass_on(&commander, &buttons, &at, &gathered, input);
        })
        .scaling(self.scaling)
        .capture_on_click(PointerCapture::Free);
        view.set_size(width, height);
        self.view = Some(view.id());
        boxed(view)
    }

    fn rebuild(&mut self) {
        self.view = None;
        let content = match self.stage.clone() {
            Stage::Choosing(problem) => self.page(self.choosing(problem.as_deref())),
            Stage::Connecting => self.page(vec![boxed(Label::new(&tr!(
                "Connecting to {computer}…",
                computer = self.title.clone()
            )))]),
            Stage::Asking(question) => self.page(self.asking(&question)),
            Stage::Viewing => self.viewing(),
        };
        self.set_content(content);
    }

    fn act(&mut self) -> bool {
        let intents: Vec<Intent> = self.intents.borrow_mut().drain(..).collect();
        let acted = !intents.is_empty();
        for intent in intents {
            match intent {
                Intent::Connect(target) => self.connect(target),
                Intent::ConnectTyped => {
                    let typed = self.typed.borrow().clone();
                    match Target::parse(&typed) {
                        Some(target) => self.connect(target),
                        None => {
                            self.stage = Stage::Choosing(Some(tr!(
                                "“{address}” isn’t an address of a computer.",
                                address = typed
                            )));
                            self.rebuild();
                        }
                    }
                }
                Intent::Answer(answer) => self.answered(answer),
                Intent::Password(password) => {
                    let user = self.user.borrow().clone();
                    self.answered(Answer::Signed(Credentials { user, password }));
                }
                Intent::Choose => {
                    self.connection = None;
                    self.stage = Stage::Choosing(None);
                    self.rebuild();
                }
            }
        }
        acted
    }

    fn answered(&mut self, answer: Answer) {
        if let Some(connection) = &self.connection {
            connection.answer(answer);
        }
        self.stage = Stage::Connecting;
        self.rebuild();
    }

    fn pump(&mut self, window: &mut Window) -> bool {
        let Some(connection) = &self.connection else {
            return false;
        };
        let events = connection.events();
        let changed = !events.is_empty();
        for event in events {
            match event {
                Event::Ask(question) => {
                    self.stage = Stage::Asking(question);
                    self.rebuild();
                }
                Event::Connected { name, encrypted } => {
                    if !name.is_empty() {
                        self.title = name;
                    }
                    self.encrypted = encrypted;
                }
                Event::Resized {
                    memory,
                    width,
                    height,
                } => self.show(window, memory, width, height),
                Event::Painted(regions) => {
                    if let Some(surface) = self.surface.as_mut()
                        && let Err(error) = surface.damage(&regions)
                    {
                        tracing::warn!("cannot show the screen: {error}");
                    }
                }
                Event::Copied(text) => {
                    self.from_server = Some(text.clone());
                    if let Err(error) = window.serve_clipboard(ClipboardRequest::Copy(text)) {
                        tracing::warn!("cannot copy from the other computer: {error}");
                    }
                }
                Event::Ended(reason) => {
                    self.connection = None;
                    self.surface = None;
                    self.memory = None;
                    self.picture = None;
                    self.stage = Stage::Choosing(reason);
                    self.title = tr!("Screen Sharing");
                    self.rebuild();
                    break;
                }
            }
        }
        changed
    }

    fn show(&mut self, window: &mut Window, memory: File, width: u32, height: u32) {
        if self.surface.is_none() {
            match window.create_external_surface() {
                Ok(surface) => self.surface = Some(surface),
                Err(error) => {
                    tracing::warn!("cannot show the screen: {error}");
                    return;
                }
            }
        }
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let layout = ShmLayout {
            width,
            height,
            stride: width * 4,
            offset: 0,
            fourcc: XRGB8888,
        };
        if let Err(error) = surface.show_shm(memory.as_fd(), layout) {
            tracing::warn!("cannot show the screen: {error}");
            return;
        }
        self.memory = Some(memory);
        match (
            self.stage == Stage::Viewing,
            self.view
                .and_then(|view| self.ui.widget_as::<SurfaceView>(view)),
        ) {
            (true, Some(view)) => {
                view.set_size(width, height);
                self.picture = Some((width, height));
            }
            _ => {
                self.picture = Some((width, height));
                self.stage = Stage::Viewing;
                self.rebuild();
                if let Err(error) = window.paste() {
                    tracing::warn!("cannot read the clipboard for the other computer: {error}");
                }
            }
        }
    }

    pub fn disconnect(&mut self) {
        self.intents.borrow_mut().push(Intent::Choose);
    }
}

impl View for Pane {
    fn scene(&mut self, window: &mut Window) -> Scene {
        window.set_title(&self.window_title());
        View::scene(&mut self.ui, window)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) -> bool {
        match &event {
            WindowEvent::ClipboardChanged if self.stage == Stage::Viewing => {
                if let Err(error) = window.paste() {
                    tracing::warn!("cannot read the clipboard for the other computer: {error}");
                }
            }
            WindowEvent::Pasted(text) if self.stage == Stage::Viewing => {
                if self.from_server.as_ref() != Some(text)
                    && let Some(connection) = &self.connection
                {
                    connection.commander().send(Command::Copy(text.clone()));
                }
            }
            _ => {}
        }
        let handled = View::event(&mut self.ui, window, event);
        let acted = self.act();
        let pumped = self.pump(window);
        if let Some(request) = self.ui.take_clipboard_request()
            && let Err(error) = window.serve_clipboard(request)
        {
            tracing::warn!("cannot copy: {error}");
        }
        handled || acted || pumped
    }

    fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.set_appearance(appearance);
    }

    fn set_active(&mut self, active: bool) {
        self.ui.set_window_active(active);
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.connection.iter().map(Connection::fd).collect()
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) -> bool {
        let pumped = self.pump(window);
        window.set_cursor(self.ui.cursor());
        pumped
    }

    fn deadline(&self) -> Option<Instant> {
        self.ui.deadline()
    }

    fn tick(&mut self, _window: &mut Window, _now: Instant) -> bool {
        true
    }

    fn animating(&self) -> bool {
        self.ui.is_animating()
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        Some(self.ui.accessibility())
    }

    fn text_input_area(&self) -> Option<Rect> {
        self.ui.text_input_area()
    }
}
