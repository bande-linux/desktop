mod pam;
mod user;

use std::cell::RefCell;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Node, Role, TreeId, TreeUpdate};
use unbound_l10n::Localizer;
use unbound_toolkit::ui::{Context, Event, SecureField, Size, Ui, Widget, WidgetId};
use unbound_toolkit::{
    Color, Item, Palette, Paragraph, Point, Rect, Scene, Shape, TextAlign, TextStyle, TextSystem,
};

pub use pam::{AuthError, authenticate};
pub use user::{User, current as current_user, display_name};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-lock.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const PAM_SERVICE: &str = "unbound-lock";
pub const SHAKE_DURATION: Duration = Duration::from_millis(480);
pub const SHAKE_AMPLITUDE: f32 = 14.0;
const SHAKES: f32 = 4.0;
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

pub fn shake_offset(elapsed: Duration) -> f32 {
    if elapsed >= SHAKE_DURATION {
        return 0.0;
    }
    let progress = elapsed.as_secs_f32() / SHAKE_DURATION.as_secs_f32();
    SHAKE_AMPLITUDE * (1.0 - progress) * (TAU * SHAKES * progress).sin()
}

pub fn clock_texts(now: &jiff::Zoned) -> (String, String) {
    (
        now.strftime("%A %-d %B").to_string(),
        now.strftime("%H:%M").to_string(),
    )
}

struct Model {
    name: String,
    initial: String,
    date: String,
    time: String,
    submitted: Option<String>,
    checking: bool,
    shake_from: Option<Instant>,
    now: Instant,
}

type Shared = Rc<RefCell<Model>>;

struct LockView {
    id: WidgetId,
    model: Shared,
    children: Vec<Box<dyn Widget>>,
    paragraphs: Option<[Paragraph; 4]>,
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
            paint: cx.palette.accent_color.into(),
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
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Window);
        node.set_label(L10N.tr_args(
            "locked",
            [("name", self.model.borrow().name.as_str().into())],
        ));
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
}

impl LockScreen {
    pub fn new(user: &User, now: &jiff::Zoned, palette: Palette, text: TextSystem) -> Self {
        let (date, time) = clock_texts(now);
        let model: Shared = Rc::new(RefCell::new(Model {
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
        }
    }

    pub fn set_clock(&mut self, now: &jiff::Zoned) {
        let (date, time) = clock_texts(now);
        let mut model = self.model.borrow_mut();
        model.date = date;
        model.time = time;
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if self.model.borrow().checking {
            return false;
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
        self.ui.perform(&ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node: self.field.into(),
            data: Some(ActionData::Value("".into())),
        });
        let mut model = self.model.borrow_mut();
        model.checking = false;
        model.shake_from = Some(at);
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
