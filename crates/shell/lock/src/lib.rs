mod backdrop;
mod pam;
mod user;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Node, Role, TreeId, TreeUpdate};
pub use unbound_datetime::DateFormats;
use unbound_l10n::Localizer;
use unbound_spring::State;
use unbound_toolkit::ui::{
    Animated, Context, Event, Key, SecureField, Size, Spring, Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle,
    TextSystem,
};

pub use backdrop::{BackdropSource, load_backdrop};
pub use pam::{AuthError, authenticate};
pub use unbound_wallpaper::{Inbox, Variant};
pub use user::{User, current as current_user, display_name};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-lock.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const PAM_SERVICE: &str = "unbound-lock";
pub const SHAKE_DURATION: Duration = Duration::from_millis(480);
const SHAKE_KICK: f64 = 600.0;
const BACKGROUND: Color = Color::hex(0x202830);
const FOREGROUND: Color = Color::hex(0xFFFFFF);
const DATE_SIZE: f32 = 20.0;
const TIME_SIZE: f32 = 96.0;
const NAME_SIZE: f32 = 15.0;
const INITIAL_SIZE: f32 = 30.0;
const AVATAR: f32 = 72.0;
const FIELD_WIDTH: f32 = 240.0;
const FIELD_FONT: f32 = 15.0;
const GAP: f32 = 12.0;
const SOURCE_SIZE: f32 = 12.0;
const MESSAGE_SIZE: f32 = 13.0;
const MESSAGE_WIDTH: f32 = 360.0;
const CAPS_LOCK: &str = "\u{21EA}";
const CAPS_LOCK_INSET: f32 = 8.0;

pub fn shake_offset(elapsed: Duration) -> f32 {
    if elapsed >= SHAKE_DURATION {
        return 0.0;
    }
    let kick = State {
        position: 0.0,
        velocity: SHAKE_KICK,
    };
    Spring::SHAKE
        .state_at(kick, 0.0, elapsed.as_secs_f64())
        .position as f32
}

pub fn clock_texts(formats: &DateFormats, now: &jiff::Zoned) -> (String, String) {
    (formats.long_date(now), now.strftime("%H:%M").to_string())
}

struct Model {
    input_source: Option<String>,
    message: Option<String>,
    caps_lock: bool,
    name: String,
    initial: String,
    date: String,
    time: String,
    submitted: Option<String>,
    checking: bool,
    shake_from: Option<Instant>,
    now: Instant,
    backdrop: Option<Image>,
    backdrop_shown: Animated,
}

type Shared = Rc<RefCell<Model>>;

struct LockView {
    id: WidgetId,
    model: Shared,
    children: Vec<Box<dyn Widget>>,
    paragraphs: Option<[Paragraph; 4]>,
    input_source: Option<Paragraph>,
    message: Option<Paragraph>,
    caps_lock: Option<Paragraph>,
    frame: Rect,
}

fn centred(paragraph: &Paragraph, centre_x: f32, y: f32) -> Point {
    Point::new((centre_x - paragraph.width() / 2.0).round(), y.round())
}

impl Widget for LockView {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let model = self.model.borrow();
        let paragraph = |text: &str, size: f32, weight: f32, cx: &mut Context| {
            cx.text.paragraph(
                text,
                TextStyle::new(size, FOREGROUND).weight(weight),
                None,
                TextAlign::Start,
            )
        };
        self.paragraphs = Some([
            paragraph(&model.date, DATE_SIZE, TextStyle::MEDIUM, cx),
            paragraph(&model.time, TIME_SIZE, TextStyle::SEMIBOLD, cx),
            paragraph(&model.initial, INITIAL_SIZE, TextStyle::MEDIUM, cx),
            paragraph(&model.name, NAME_SIZE, TextStyle::SEMIBOLD, cx),
        ]);
        self.input_source = model.input_source.as_ref().map(|name| {
            let hint = L10N.tr_args("input-source", [("name", name.as_str().into())]);
            paragraph(&hint, SOURCE_SIZE, TextStyle::MEDIUM, cx)
        });
        self.message = model.message.as_ref().map(|message| {
            cx.text.paragraph(
                message,
                TextStyle::new(MESSAGE_SIZE, FOREGROUND).weight(TextStyle::MEDIUM),
                Some(MESSAGE_WIDTH),
                TextAlign::Center,
            )
        });
        self.caps_lock = model.caps_lock.then(|| {
            cx.text.paragraph(
                CAPS_LOCK,
                TextStyle::new(FIELD_FONT, cx.palette.ink2).weight(TextStyle::SEMIBOLD),
                None,
                TextAlign::Start,
            )
        });
        drop(model);
        for child in &mut self.children {
            child.measure(cx, Size::new(FIELD_WIDTH, available.height));
        }
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let model = self.model.borrow();
        let shake = model.shake_from.map_or(0.0, |from| {
            shake_offset(model.now.saturating_duration_since(from))
        });
        drop(model);
        let name_bottom = self.avatar().y + AVATAR + GAP + self.name_height();
        let x = frame.x + ((frame.width - FIELD_WIDTH) / 2.0).round() + shake.round();
        for child in &mut self.children {
            child.place(
                cx,
                Rect::new(x, name_bottom + GAP, FIELD_WIDTH, frame.height),
            );
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        items.push(Item::Fill {
            shape: Shape::rect(self.frame),
            paint: BACKGROUND.into(),
        });
        let model = self.model.borrow();
        if let Some(image) = &model.backdrop {
            let (alpha, _) = model.backdrop_shown.value_at(model.now);
            items.push(Item::Opacity {
                alpha,
                items: vec![Item::Image {
                    rect: self.frame,
                    image: image.clone(),
                }],
            });
        }
        drop(model);
        let Some([date, time, initial, name]) = &self.paragraphs else {
            return;
        };
        let centre = self.frame.x + self.frame.width / 2.0;
        let top = self.frame.y + self.frame.height * 0.1;
        items.push(Item::Text {
            origin: centred(date, centre, top),
            paragraph: date.clone(),
        });
        items.push(Item::Text {
            origin: centred(time, centre, top + date.height()),
            paragraph: time.clone(),
        });
        let avatar = self.avatar();
        items.push(Item::Fill {
            shape: Shape::rounded(avatar, AVATAR / 2.0),
            paint: cx.palette.accent.into(),
        });
        items.push(Item::Text {
            origin: centred(
                initial,
                centre,
                avatar.y + (AVATAR - initial.height()) / 2.0,
            ),
            paragraph: initial.clone(),
        });
        items.push(Item::Text {
            origin: centred(name, centre, avatar.y + AVATAR + GAP),
            paragraph: name.clone(),
        });
        for child in &self.children {
            child.paint(cx, items);
        }
        let Some(field) = self.children.first().map(|field| field.frame()) else {
            return;
        };
        if let Some(caps_lock) = &self.caps_lock {
            items.push(Item::Text {
                origin: Point::new(
                    (field.x + field.width - CAPS_LOCK_INSET - caps_lock.width()).round(),
                    (field.y + (field.height - caps_lock.height()) / 2.0).round(),
                ),
                paragraph: caps_lock.clone(),
            });
        }
        let mut below = field.y + field.height + GAP;
        if let Some(message) = &self.message {
            items.push(Item::Text {
                origin: Point::new((centre - MESSAGE_WIDTH / 2.0).round(), below.round()),
                paragraph: message.clone(),
            });
            below += message.height() + GAP / 2.0;
        }
        if let Some(source) = &self.input_source {
            items.push(Item::Text {
                origin: centred(source, centre, below),
                paragraph: source.clone(),
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Window);
        let model = self.model.borrow();
        let locked = L10N.tr_args("locked", [("name", model.name.as_str().into())]);
        let input_source = model
            .input_source
            .as_ref()
            .map(|name| L10N.tr_args("input-source", [("name", name.as_str().into())]));
        let caps_lock = model.caps_lock.then(|| L10N.tr("caps-lock"));
        let label = [Some(locked), model.message.clone(), caps_lock, input_source]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        node.set_label(label);
        node
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}

impl LockView {
    fn avatar(&self) -> Rect {
        Rect::new(
            self.frame.x + ((self.frame.width - AVATAR) / 2.0).round(),
            self.frame.y + (self.frame.height * 0.58).round(),
            AVATAR,
            AVATAR,
        )
    }

    fn name_height(&self) -> f32 {
        self.paragraphs
            .as_ref()
            .map_or(NAME_SIZE, |[_, _, _, name]| name.height())
    }
}

pub struct LockScreen {
    model: Shared,
    ui: Ui,
    field: WidgetId,
    formats: DateFormats,
}

impl LockScreen {
    pub fn new(
        user: &User,
        formats: DateFormats,
        now: &jiff::Zoned,
        palette: Palette,
        text: TextSystem,
    ) -> Self {
        let (date, time) = clock_texts(&formats, now);
        let model: Shared = Rc::new(RefCell::new(Model {
            input_source: None,
            message: None,
            caps_lock: false,
            name: user.full_name.clone(),
            initial: user
                .full_name
                .chars()
                .next()
                .map(|initial| initial.to_uppercase().collect())
                .unwrap_or_default(),
            date,
            time,
            submitted: None,
            checking: false,
            shake_from: None,
            now: Instant::now(),
            backdrop: None,
            backdrop_shown: Animated::new(0.0),
        }));
        let submitted = model.clone();
        let field = SecureField::new(&L10N.tr("enter-password"), FIELD_FONT, move |password| {
            let mut model = submitted.borrow_mut();
            if !model.checking && !password.is_empty() {
                model.checking = true;
                model.submitted = Some(password.to_owned());
            }
        });
        let field_id = field.id();
        let view = LockView {
            id: WidgetId::next(),
            model: model.clone(),
            children: vec![Box::new(field)],
            paragraphs: None,
            input_source: None,
            message: None,
            caps_lock: None,
            frame: Rect::default(),
        };
        let mut ui = Ui::new(Box::new(view), Context::new(text, palette));
        ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: field_id.into(),
            data: None,
        });
        Self {
            model,
            ui,
            field: field_id,
            formats,
        }
    }

    pub fn set_palette(&mut self, palette: Palette) {
        self.ui.context().palette = palette;
    }

    pub fn set_input_source(&mut self, name: Option<String>) -> bool {
        let mut model = self.model.borrow_mut();
        let changed = model.input_source != name;
        model.input_source = name;
        changed
    }

    pub fn set_caps_lock(&mut self, on: bool) -> bool {
        let mut model = self.model.borrow_mut();
        let changed = model.caps_lock != on;
        model.caps_lock = on;
        changed
    }

    pub fn set_message(&mut self, message: Option<String>) {
        self.model.borrow_mut().message = message;
    }

    pub fn ask(&mut self, prompt: Option<&str>) {
        let placeholder = prompt
            .map(|prompt| prompt.trim().trim_end_matches(':').to_owned())
            .unwrap_or_else(|| L10N.tr("enter-password"));
        if let Some(field) = self.ui.widget_as::<SecureField>(self.field) {
            field.set_placeholder(&placeholder);
        }
        self.clear_field();
        self.model.borrow_mut().checking = false;
    }

    fn clear_field(&mut self) {
        self.ui.perform(&ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node: self.field.into(),
            data: Some(ActionData::Value("".into())),
        });
    }

    pub fn set_clock(&mut self, now: &jiff::Zoned) {
        let (date, time) = clock_texts(&self.formats, now);
        let mut model = self.model.borrow_mut();
        model.date = date;
        model.time = time;
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if self.model.borrow().checking {
            return false;
        }
        if let Event::Key {
            key: Key::Escape, ..
        } = event
        {
            self.clear_field();
            self.model.borrow_mut().message = None;
            return true;
        }
        self.ui.handle(event)
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        if self.model.borrow().checking {
            return false;
        }
        self.ui.perform(request)
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn take_password(&mut self) -> Option<String> {
        self.model.borrow_mut().submitted.take()
    }

    pub fn reject(&mut self, at: Instant) {
        self.clear_field();
        let mut model = self.model.borrow_mut();
        model.checking = false;
        model.shake_from = Some(at);
    }

    pub fn set_backdrop(&mut self, image: Image, now: Instant) {
        let mut model = self.model.borrow_mut();
        model.backdrop = Some(image);
        model
            .backdrop_shown
            .animate_at(now, false, 1.0, Spring::GENTLE);
    }

    pub fn is_fading(&self, now: Instant) -> bool {
        self.model.borrow().backdrop_shown.value_at(now).1
    }

    pub fn shaking_until(&self) -> Option<Instant> {
        let model = self.model.borrow();
        model
            .shake_from
            .map(|from| from + SHAKE_DURATION)
            .filter(|end| *end > model.now)
    }

    pub fn field_frame(&mut self, width: u32, height: u32) -> Rect {
        self.ui.layout(Size::new(width as f32, height as f32));
        self.ui
            .widget(self.field)
            .map(|field| field.frame())
            .unwrap_or_default()
    }

    pub fn advance(&mut self, now: Instant) {
        self.model.borrow_mut().now = now;
    }

    pub fn scene(&mut self, width: u32, height: u32) -> Scene {
        self.ui.scene(width, height)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_message_id_used_in_code_is_in_the_english_file() {
        let sources = [include_str!("lib.rs"), include_str!("main.rs")];
        assert_eq!(
            unbound_l10n::missing_ids(super::ENGLISH, &sources),
            Vec::<&str>::new()
        );
    }
}
