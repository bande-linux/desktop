mod backdrop;
mod devices;
mod pam;
mod user;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Node, Role, TreeId, TreeUpdate};
use unbound_bar::symbols::{
    Symbol, Symbols, battery_items, natural_size, place_symbol, snap, wifi_symbol,
};
pub use unbound_datetime::DateFormats;
use unbound_l10n::Localizer;
use unbound_spring::State;
pub use unbound_status::devices::PowerAction;
use unbound_status::session::Ending;
use unbound_status::status::{Battery, Wifi};
use unbound_toolkit::ui::{
    Animated, Button, Context, Event, Key, SecureField, Size, Spring, Ui, Widget, WidgetId,
};
use unbound_toolkit::{
    Appearance, Image, Item, Palette, Paragraph, Point, Rect, Scene, Scheme, Shape, TextAlign,
    TextRole, TextStyle, TextSystem,
};
use zeroize::Zeroizing;

pub use backdrop::{BackdropSource, Look, load_backdrop};
pub use devices::Devices;
pub use pam::{AuthError, authenticate};
pub use unbound_inbox::Inbox;
pub use unbound_wallpaper::Variant;
pub use user::{User, current as current_user, display_name};

const ENGLISH: &str = include_str!("../locales/en-US/unbound-lock.ftl");

pub static L10N: LazyLock<Localizer> = LazyLock::new(|| Localizer::new(&[("en-US", ENGLISH)]));

pub const PAM_SERVICE: &str = "unbound-lock";
pub const SHAKE_DURATION: Duration = Duration::from_millis(480);
const SHAKE_KICK: f64 = 600.0;
const TIME_SIZE: f32 = 96.0;
const TIME_WEIGHT: f32 = 300.0;
const INITIAL_SIZE: f32 = 30.0;
const AVATAR: f32 = 72.0;
const FIELD_WIDTH: f32 = 240.0;
const FIELD_FONT: f32 = 15.0;
const GAP: f32 = 12.0;
const MESSAGE_WIDTH: f32 = 360.0;
const PEER: f32 = 6.0;
const PEER_GAP: f32 = 6.0;
const STATUS_TOP: f32 = 12.0;
const STATUS_HEIGHT: f32 = 24.0;
const STATUS_MARGIN: f32 = 16.0;
const STATUS_GAP: f32 = 10.0;
const BATTERY_SLOT: f32 = 27.0;
const PERCENT_GAP: f32 = 4.0;
const POWER_BOTTOM: f32 = 48.0;
const POWER_GAP: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerPress {
    Ask(PowerAction),
    Cancel,
    Confirm,
}

fn ending_of(action: PowerAction) -> Option<Ending> {
    match action {
        PowerAction::Sleep => None,
        PowerAction::Restart => Some(Ending::Restart),
        PowerAction::ShutDown => Some(Ending::ShutDown),
    }
}

pub fn night(appearance: Appearance) -> Palette {
    Appearance {
        scheme: Scheme::Dark,
        ..appearance
    }
    .palette()
}

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
    (formats.long_date(now), formats.time(now).0)
}

struct Model {
    input_source: Option<String>,
    message: Option<String>,
    position: (usize, usize),
    name: String,
    initial: String,
    date: String,
    time: String,
    submitted: Option<Zeroizing<String>>,
    checking: bool,
    shake_from: Option<Instant>,
    now: Instant,
    backdrop: Option<Image>,
    backdrop_shown: Animated,
    battery: Option<Battery>,
    wifi: Option<Symbol>,
    symbols: Symbols,
    scale: f64,
    powers: Vec<PowerAction>,
    asking: Option<PowerAction>,
    power_press: Option<PowerPress>,
    wallpaper: Option<Image>,
    content_shown: Animated,
}

type Shared = Rc<RefCell<Model>>;

struct LockView {
    id: WidgetId,
    model: Shared,
    children: Vec<Box<dyn Widget>>,
    paragraphs: Option<[Paragraph; 4]>,
    input_source: Option<Paragraph>,
    message: Option<Paragraph>,
    percent: Option<Paragraph>,
    row: Vec<Size>,
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
        let palette = cx.palette;
        let paragraph = |text: &str, style: TextStyle, cx: &mut Context| {
            cx.text.paragraph(text, style, None, TextAlign::Start)
        };
        self.paragraphs = Some([
            paragraph(
                &model.date,
                TextStyle::role(TextRole::Heading, palette.ink),
                cx,
            ),
            paragraph(
                &model.time,
                TextStyle::new(TIME_SIZE, palette.ink)
                    .weight(TIME_WEIGHT)
                    .tabular(),
                cx,
            ),
            paragraph(
                &model.initial,
                TextStyle::new(INITIAL_SIZE, palette.on_accent).weight(TextStyle::BOLD),
                cx,
            ),
            paragraph(
                &model.name,
                TextStyle::role(TextRole::WindowTitle, palette.ink),
                cx,
            ),
        ]);
        self.input_source = model.input_source.as_ref().map(|name| {
            let hint = L10N.tr_args("input-source", [("name", name.as_str().into())]);
            paragraph(&hint, TextStyle::role(TextRole::Callout, palette.ink2), cx)
        });
        self.percent = model.battery.map(|battery| {
            let percent = L10N.tr_args(
                "battery-percent",
                [("percent", battery.percent.round().into())],
            );
            paragraph(
                &percent,
                TextStyle::role(TextRole::Callout, palette.ink),
                cx,
            )
        });
        self.message = model.message.as_ref().map(|message| {
            cx.text.paragraph(
                message,
                TextStyle::role(TextRole::BodyStrong, palette.ink),
                Some(MESSAGE_WIDTH),
                TextAlign::Center,
            )
        });
        drop(model);
        let mut sizes: Vec<Size> = Vec::new();
        for child in &mut self.children {
            sizes.push(child.measure(cx, Size::new(FIELD_WIDTH, available.height)));
        }
        self.row = sizes.split_off(1.min(sizes.len()));
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let model = self.model.borrow();
        let shake = model.shake_from.map_or(0.0, |from| {
            shake_offset(model.now.saturating_duration_since(from))
        });
        drop(model);
        let name_bottom = self.peers_top() + self.peers_height();
        let x = frame.x + ((frame.width - FIELD_WIDTH) / 2.0).round() + shake.round();
        let first = self.children.len().min(1);
        let (field, row) = self.children.split_at_mut(first);
        for child in field {
            child.place(
                cx,
                Rect::new(x, name_bottom + GAP, FIELD_WIDTH, frame.height),
            );
        }
        let width = self
            .row
            .iter()
            .map(|size| size.width + POWER_GAP)
            .sum::<f32>()
            - POWER_GAP;
        let height = self.row.iter().map(|size| size.height).fold(0.0, f32::max);
        let mut left = frame.x + ((frame.width - width) / 2.0).round();
        let top = frame.y + frame.height - POWER_BOTTOM - height;
        for (child, size) in row.iter_mut().zip(&self.row) {
            child.place(cx, Rect::new(left, top, size.width, size.height));
            left += size.width + POWER_GAP;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let backdrop = Shape::rect(self.frame);
        items.push(Item::Fill {
            shape: backdrop,
            paint: cx.palette.chassis.into(),
        });
        let model = self.model.borrow();
        let (shown, _) = model.content_shown.value_at(model.now);
        let unveiling = match &model.wallpaper {
            Some(wallpaper) if model.content_shown.target() < 1.0 => {
                items.push(Item::Image {
                    rect: self.frame,
                    image: wallpaper.clone(),
                });
                shown
            }
            _ => 1.0,
        };
        if let Some(image) = &model.backdrop {
            let (alpha, _) = model.backdrop_shown.value_at(model.now);
            items.push(Item::Opacity {
                alpha: alpha * unveiling,
                items: vec![
                    Item::Image {
                        rect: self.frame,
                        image: image.clone(),
                    },
                    Item::Fill {
                        shape: backdrop,
                        paint: cx.palette.scrim.into(),
                    },
                ],
            });
        }
        drop(model);
        let mut content = Vec::new();
        self.paint_content(cx, &mut content);
        self.paint_status(cx, &mut content);
        match shown >= 1.0 {
            true => items.extend(content),
            false => items.push(Item::Opacity {
                alpha: shown.max(0.0),
                items: content,
            }),
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
        let (current, count) = model.position;
        let position = (count > 1).then(|| {
            L10N.tr_args(
                "user-position",
                [("index", (current + 1).into()), ("count", count.into())],
            )
        });
        let battery = model.battery.map(|battery| {
            L10N.tr_args(
                "battery-level",
                [("percent", battery.percent.round().into())],
            )
        });
        let label = [
            Some(locked),
            position,
            model.message.clone(),
            input_source,
            battery,
        ]
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
    fn paint_content(&self, cx: &Context, items: &mut Vec<Item>) {
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
        let (current, count) = self.model.borrow().position;
        if count > 1 {
            let row = count as f32 * (PEER + PEER_GAP) - PEER_GAP;
            let left = (centre - row / 2.0).round();
            let top = self.peers_top() + GAP / 2.0;
            for index in 0..count {
                let square = Shape::rect(Rect::new(
                    left + index as f32 * (PEER + PEER_GAP),
                    top,
                    PEER,
                    PEER,
                ));
                items.push(if index == current {
                    Item::Fill {
                        shape: square,
                        paint: cx.palette.ink.into(),
                    }
                } else {
                    Item::Stroke {
                        shape: Shape::rect(square.rect.inset(0.5)),
                        width: 1.0,
                        paint: cx.palette.ink2.into(),
                    }
                });
            }
        }
        for child in &self.children {
            child.paint(cx, items);
        }
        let Some(field) = self.children.first().map(|field| field.frame()) else {
            return;
        };
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

    fn paint_status(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut model = self.model.borrow_mut();
        let model = &mut *model;
        let scale = model.scale;
        let top = self.frame.y + STATUS_TOP;
        let mut right = self.frame.x + self.frame.width - STATUS_MARGIN;
        if let Some(battery) = model.battery {
            let icon = Rect::new(right - BATTERY_SLOT, top, BATTERY_SLOT, STATUS_HEIGHT);
            battery_items(&mut model.symbols, &cx.palette, battery, icon, scale, items);
            right = icon.x;
            if let Some(percent) = &self.percent {
                let x = right - PERCENT_GAP - percent.width();
                items.push(Item::Text {
                    origin: Point::new(
                        snap(x, scale),
                        snap(top + (STATUS_HEIGHT - percent.height()) / 2.0, scale),
                    ),
                    paragraph: percent.clone(),
                });
                right = x;
            }
            right -= STATUS_GAP;
        }
        if let Some(symbol) = model.wifi {
            let (width, _) = natural_size(symbol);
            let rect = Rect::new(right - width, top, width, STATUS_HEIGHT);
            place_symbol(
                &mut model.symbols,
                symbol,
                cx.palette.ink,
                rect,
                scale,
                items,
            );
        }
    }

    fn avatar(&self) -> Rect {
        Rect::new(
            self.frame.x + ((self.frame.width - AVATAR) / 2.0).round(),
            self.frame.y + (self.frame.height * 0.58).round(),
            AVATAR,
            AVATAR,
        )
    }

    fn peers_top(&self) -> f32 {
        let name = self
            .paragraphs
            .as_ref()
            .map_or(0.0, |[_, _, _, name]| name.height());
        self.avatar().y + AVATAR + GAP + name
    }

    fn peers_height(&self) -> f32 {
        match self.model.borrow().position {
            (_, 0 | 1) => 0.0,
            _ => GAP / 2.0 + PEER,
        }
    }
}

pub struct LockScreen {
    model: Shared,
    ui: Ui,
    view: WidgetId,
    field: WidgetId,
    formats: DateFormats,
}

impl LockScreen {
    pub fn new(
        user: &User,
        formats: DateFormats,
        now: &jiff::Zoned,
        appearance: Appearance,
        text: TextSystem,
    ) -> Self {
        let (date, time) = clock_texts(&formats, now);
        let model: Shared = Rc::new(RefCell::new(Model {
            input_source: None,
            message: None,
            position: (0, 1),
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
            battery: None,
            wifi: None,
            symbols: Symbols::default(),
            scale: 1.0,
            powers: Vec::new(),
            asking: None,
            power_press: None,
            wallpaper: None,
            content_shown: Animated::new(1.0),
        }));
        let submitted = model.clone();
        let field = SecureField::new(&L10N.tr("enter-password"), FIELD_FONT, move |password| {
            let mut model = submitted.borrow_mut();
            if !model.checking && !password.is_empty() {
                model.checking = true;
                model.submitted = Some(Zeroizing::new(password.to_owned()));
            }
        });
        let field_id = field.id();
        let view_id = WidgetId::next();
        let view = LockView {
            id: view_id,
            model: model.clone(),
            children: vec![Box::new(field)],
            paragraphs: None,
            input_source: None,
            message: None,
            percent: None,
            row: Vec::new(),
            frame: Rect::default(),
        };
        let mut ui = Ui::new(Box::new(view), Context::new(text, night(appearance)));
        ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: field_id.into(),
            data: None,
        });
        Self {
            model,
            ui,
            view: view_id,
            field: field_id,
            formats,
        }
    }

    pub fn offer_power(&mut self, powers: &[PowerAction]) {
        self.model.borrow_mut().powers = powers.to_vec();
        self.ui.context().keyboard_navigation = true;
        self.show_power_row();
    }

    fn power_button(&self, title: &str, press: PowerPress) -> Button {
        let model = self.model.clone();
        Button::new(title, move || model.borrow_mut().power_press = Some(press))
    }

    fn show_power_row(&mut self) {
        let (powers, asking) = {
            let model = self.model.borrow();
            (model.powers.clone(), model.asking)
        };
        let row: Vec<Button> = match asking.and_then(ending_of) {
            Some(ending) => vec![
                self.power_button(&L10N.tr("cancel"), PowerPress::Cancel)
                    .cancel(),
                self.power_button(&ending.button(), PowerPress::Confirm)
                    .primary(),
            ],
            None => powers
                .into_iter()
                .map(|power| {
                    let title = ending_of(power).map_or_else(|| L10N.tr("sleep"), Ending::button);
                    self.power_button(&title, PowerPress::Ask(power))
                })
                .collect(),
        };
        let focus = asking.and_then(|_| row.last().map(Widget::id));
        if let Some(view) = self.ui.widget_as::<LockView>(self.view) {
            view.children.truncate(1);
            view.children.extend(
                row.into_iter()
                    .map(|button| Box::new(button) as Box<dyn Widget>),
            );
        }
        let target = focus.unwrap_or(self.field);
        self.ui.perform(&ActionRequest {
            action: Action::Focus,
            target_tree: TreeId::ROOT,
            target_node: target.into(),
            data: None,
        });
    }

    fn ask_power(&mut self, asking: Option<PowerAction>) {
        let mut model = self.model.borrow_mut();
        model.asking = asking;
        model.message = asking.and_then(ending_of).map(Ending::question);
        drop(model);
        self.show_power_row();
    }

    pub fn take_power(&mut self) -> Option<PowerAction> {
        let press = self.model.borrow_mut().power_press.take()?;
        match press {
            PowerPress::Ask(PowerAction::Sleep) => Some(PowerAction::Sleep),
            PowerPress::Ask(power) => {
                self.ask_power(Some(power));
                None
            }
            PowerPress::Cancel => {
                self.ask_power(None);
                None
            }
            PowerPress::Confirm => {
                let power = self.model.borrow().asking;
                self.ask_power(None);
                power
            }
        }
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.ui.context().palette = night(appearance);
    }

    pub fn set_input_source(&mut self, name: Option<String>) -> bool {
        let mut model = self.model.borrow_mut();
        let changed = model.input_source != name;
        model.input_source = name;
        changed
    }

    pub fn set_caps_lock(&mut self, on: bool) -> bool {
        let cx = self.ui.context();
        let changed = cx.caps_lock != on;
        cx.caps_lock = on;
        changed
    }

    pub fn set_devices(&mut self, battery: Option<Battery>, wifi: Option<&Wifi>) -> bool {
        let mut model = self.model.borrow_mut();
        let wifi = wifi.map(|wifi| wifi_symbol(Some(wifi)));
        let changed = model.battery != battery || model.wifi != wifi;
        model.battery = battery;
        model.wifi = wifi;
        changed
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.model.borrow_mut().scale = scale;
    }

    pub fn set_position(&mut self, index: usize, count: usize) {
        self.model.borrow_mut().position = (index, count);
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
            if self.model.borrow().asking.is_some() {
                self.ask_power(None);
                return true;
            }
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

    pub fn take_password(&mut self) -> Option<Zeroizing<String>> {
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
            .animate_at(now, false, 1.0, Spring::FADE);
    }

    pub fn set_wallpaper(&mut self, image: Image) {
        self.model.borrow_mut().wallpaper = Some(image);
    }

    pub fn leave(&mut self, now: Instant) {
        self.model
            .borrow_mut()
            .content_shown
            .animate_at(now, false, 0.0, Spring::STANDARD);
    }

    pub fn has_left(&self) -> bool {
        let model = self.model.borrow();
        let (shown, moving) = model.content_shown.value_at(model.now);
        shown <= 0.0 && !moving
    }

    pub fn is_animating(&self) -> bool {
        let model = self.model.borrow();
        model.backdrop_shown.value_at(model.now).1
            || model.content_shown.value_at(model.now).1
            || model
                .shake_from
                .is_some_and(|from| model.now < from + SHAKE_DURATION)
            || self.ui.is_animating()
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
