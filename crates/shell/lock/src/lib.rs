mod backdrop;
mod choices;
mod conversation;
mod devices;
mod pam;
mod user;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, Node, Role, TreeId, TreeUpdate};
use bande_bar::symbols::{
    Symbol, Symbols, battery_items, natural_size, place_symbol, snap, wifi_symbol,
};
pub use bande_datetime::DateFormats;
use bande_l10n::tr;
use bande_spring::State;
pub use bande_status::devices::PowerAction;
use bande_status::session::Ending;
use bande_status::status::{Battery, Wifi};
use bande_toolkit::ui::{
    Animated, Button, Context, Event, Key, ProgressBar, SecureField, Size, Spring, Ui, Widget,
    WidgetId,
};
use bande_toolkit::{
    Appearance, Color, Image, Item, Palette, Paragraph, Point, Rect, Scene, Scheme, Shape,
    TextAlign, TextRole, TextStyle, TextSystem,
};
use bande_xdg::{data_dirs, find_icon};
use zeroize::Zeroizing;

pub use backdrop::{BackdropSource, Look, load_backdrop, load_pictures};
pub use bande_inbox::Inbox;
pub use bande_wallpaper::{Fit, Variant};
pub use choices::Peer;
use choices::{SourceButton, UserList};
pub use conversation::{Backoff, Messages, Prompt, Refusal, clip_message};
pub use devices::{Corner, Devices};
pub use pam::{Accepted, Said, Transaction};
pub use user::{User, current as current_user, display_name};

pub const PAM_SERVICE: &str = "bande-lock";
pub const SHAKE_DURATION: Duration = Duration::from_millis(480);
const SHAKE_KICK: f64 = 600.0;
const CHECKING_SHOWN_AFTER: Duration = Duration::from_millis(200);
const TIME_SIZE: f32 = 112.0;
const TIME_WEIGHT: f32 = 300.0;
const CLOCK_TOP: f32 = 0.11;
const IDENTITY_TOP: f32 = 0.56;
const INITIAL_SIZE: f32 = 36.0;
const AVATAR: f32 = 88.0;
const AVATAR_RADIUS: f32 = 20.0;
pub const PICTURE_PIXELS: u32 = 264;
const FIELD_WIDTH: f32 = 280.0;
const FIELD_FONT: f32 = 15.0;
const GAP: f32 = 12.0;
const NAME_GAP: f32 = 14.0;
const FIELD_GAP: f32 = 16.0;
const MESSAGE_WIDTH: f32 = 360.0;
const USERS_GAP: f32 = 24.0;
const STATUS_TOP: f32 = 12.0;
const STATUS_HEIGHT: f32 = 24.0;
const STATUS_MARGIN: f32 = 16.0;
const STATUS_GAP: f32 = 10.0;
const BATTERY_SLOT: f32 = 27.0;
const PERCENT_GAP: f32 = 4.0;
const POWER_BOTTOM: f32 = 40.0;
const POWER_GAP: f32 = 12.0;
const POWER_SYMBOL_GAP: f32 = 40.0;
const POWER_BUTTON: f32 = 44.0;
const POWER_ICON_PIXELS: u32 = 48;
const POWER_LABEL_GAP: f32 = 4.0;
const FIELD_INDEX: usize = 0;
const PROGRESS_INDEX: usize = 1;
const SOURCE_INDEX: usize = 2;
const USERS_INDEX: usize = 3;
const POWER_INDEX: usize = 4;
pub const RESTS_AFTER: Duration = Duration::from_secs(60);
const BACKOFF_TICK: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Lock,
    Login,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PowerPress {
    Ask(PowerAction),
    Cancel,
    Confirm,
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
    purpose: Purpose,
    input_source: Option<String>,
    question: Option<String>,
    messages: Messages,
    backoff: Backoff,
    cancel: bool,
    note: String,
    position: (usize, usize),
    name: String,
    initial: String,
    picture: Option<Image>,
    date: String,
    time: String,
    submitted: Option<Zeroizing<String>>,
    checking: Option<Instant>,
    shake_from: Option<Instant>,
    now: Instant,
    backdrop: Option<Image>,
    backdrop_shown: Animated,
    battery: Option<Battery>,
    network: Option<(Symbol, String)>,
    symbols: Symbols,
    scale: f64,
    powers: Vec<PowerAction>,
    asking: Option<PowerAction>,
    power_press: Option<PowerPress>,
    wallpaper: Option<Image>,
    content_shown: Animated,
    identity_shown: Animated,
    resting: bool,
    rests_at: Option<Instant>,
    power_labels: Vec<String>,
}

impl Model {
    fn shown_message(&self) -> Option<String> {
        if let Some(question) = &self.question {
            return Some(question.clone());
        }
        let notice = self.backoff.notice(self.now);
        match (self.messages.text(), notice) {
            (Some(messages), Some(notice)) => Some(format!("{messages}\n{notice}")),
            (messages, notice) => messages.or(notice),
        }
    }
}

type Shared = Rc<RefCell<Model>>;

struct LockView {
    id: WidgetId,
    model: Shared,
    children: Vec<Box<dyn Widget>>,
    paragraphs: Option<[Paragraph; 4]>,
    message: Option<Paragraph>,
    note: Option<Paragraph>,
    percent: Option<Paragraph>,
    power_labels: Vec<Paragraph>,
    row: Vec<Size>,
    extras: [Size; 2],
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
                TextStyle::role(TextRole::Title, palette.ink).weight(TextStyle::MEDIUM),
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
                TextStyle::role(TextRole::Heading, palette.ink),
                cx,
            ),
        ]);
        self.percent = model.battery.map(|battery| {
            let percent = tr!("{percent}%", percent = battery.percent.round());
            paragraph(
                &percent,
                TextStyle::role(TextRole::Callout, palette.ink),
                cx,
            )
        });
        self.message = model.shown_message().map(|message| {
            cx.text.paragraph(
                &message,
                TextStyle::role(TextRole::BodyStrong, palette.ink).weight(TextStyle::MEDIUM),
                Some(MESSAGE_WIDTH),
                TextAlign::Center,
            )
        });
        self.note = (!model.note.is_empty()).then(|| {
            cx.text.paragraph(
                &model.note,
                TextStyle::role(TextRole::Callout, palette.ink),
                Some(MESSAGE_WIDTH),
                TextAlign::Center,
            )
        });
        self.power_labels = model
            .power_labels
            .iter()
            .map(|label| paragraph(label, TextStyle::role(TextRole::Callout, palette.ink), cx))
            .collect();
        drop(model);
        let mut sizes: Vec<Size> = Vec::new();
        for (index, child) in self.children.iter_mut().enumerate() {
            let room = match index {
                SOURCE_INDEX | USERS_INDEX => available,
                _ => Size::new(FIELD_WIDTH, available.height),
            };
            sizes.push(child.measure(cx, room));
        }
        self.row = sizes.split_off(POWER_INDEX.min(sizes.len()));
        self.extras =
            [SOURCE_INDEX, USERS_INDEX].map(|index| sizes.get(index).copied().unwrap_or_default());
        available
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let model = self.model.borrow();
        let shake = model.shake_from.map_or(0.0, |from| {
            shake_offset(model.now.saturating_duration_since(from))
        });
        drop(model);
        let name_bottom = self.name_bottom();
        let x = frame.x + ((frame.width - FIELD_WIDTH) / 2.0).round() + shake.round();
        let field_top = name_bottom + FIELD_GAP;
        let status_left = self.status_left();
        let power_top = self.power_top(frame);
        let label_room = self.label_room();
        let scale = self.model.borrow().scale;
        let first = self.children.len().min(POWER_INDEX);
        let (above, row) = self.children.split_at_mut(first);
        if let [field, progress, source, users] = above {
            field.place(cx, Rect::new(x, field_top, FIELD_WIDTH, frame.height));
            let centre_left = frame.x + ((frame.width - FIELD_WIDTH) / 2.0).round();
            let below = field.frame().y + field.frame().height + GAP;
            progress.place(cx, Rect::new(centre_left, below, FIELD_WIDTH, frame.height));
            let [source_size, users_size] = self.extras;
            source.place(
                cx,
                Rect::new(
                    snap(status_left - source_size.width, scale),
                    frame.y + STATUS_TOP + ((STATUS_HEIGHT - source_size.height) / 2.0).round(),
                    source_size.width,
                    source_size.height,
                ),
            );
            users.place(
                cx,
                Rect::new(
                    frame.x + ((frame.width - users_size.width) / 2.0).round(),
                    power_top - USERS_GAP - users_size.height,
                    users_size.width,
                    users_size.height,
                ),
            );
        }
        let symbols = !self.power_labels.is_empty();
        let (button, gap) = match symbols {
            true => (Size::new(POWER_BUTTON, POWER_BUTTON), POWER_SYMBOL_GAP),
            false => (Size::default(), POWER_GAP),
        };
        let widths: Vec<f32> = self
            .row
            .iter()
            .enumerate()
            .map(|(index, size)| match symbols {
                true => self
                    .power_labels
                    .get(index)
                    .map_or(button.width, |label| label.width().max(button.width)),
                false => size.width,
            })
            .collect();
        let width = widths.iter().map(|width| width + gap).sum::<f32>() - gap;
        let height = match symbols {
            true => button.height,
            false => self.row.iter().map(|size| size.height).fold(0.0, f32::max),
        };
        let mut left = frame.x + ((frame.width - width) / 2.0).round();
        let top = frame.y + frame.height - POWER_BOTTOM - label_room - height;
        for ((child, size), slot) in row.iter_mut().zip(&self.row).zip(&widths) {
            let shown = match symbols {
                true => button,
                false => *size,
            };
            let inset = ((slot - shown.width) / 2.0).round();
            child.place(cx, Rect::new(left + inset, top, shown.width, shown.height));
            left += slot + gap;
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        self.paint_screen(cx, Shape::rect(self.frame), items);
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Window);
        let model = self.model.borrow();
        let screen = match model.purpose {
            Purpose::Lock => tr!("Locked. {name}", name = model.name.as_str()),
            Purpose::Login => tr!("Sign in. {name}", name = model.name.as_str()),
        };
        let input_source = model
            .input_source
            .as_ref()
            .map(|name| tr!("Input source {name}", name = name.as_str()));
        let network = model.network.as_ref().map(|(_, label)| label.clone());
        let (current, count) = model.position;
        let position = (count > 1).then(|| {
            tr!(
                "User {index} of {count}. Up and Down choose another.",
                index = (current + 1),
                count = count
            )
        });
        let battery = model
            .battery
            .map(|battery| tr!("Battery {percent}%", percent = battery.percent.round()));
        let label = [
            Some(screen),
            position,
            model.shown_message(),
            input_source,
            network,
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
    fn paint_screen(&self, cx: &Context, backdrop: Shape, items: &mut Vec<Item>) {
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

    fn paint_content(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some([date, time, _, _]) = &self.paragraphs else {
            return;
        };
        let centre = self.frame.x + self.frame.width / 2.0;
        let top = self.frame.y + self.frame.height * CLOCK_TOP;
        items.push(Item::Text {
            origin: centred(date, centre, top),
            paragraph: date.clone(),
        });
        items.push(Item::Text {
            origin: centred(time, centre, top + date.height()),
            paragraph: time.clone(),
        });
        if let Some(note) = &self.note {
            let bottom = self
                .children
                .get(POWER_INDEX)
                .map_or(self.frame.y + self.frame.height - POWER_BOTTOM, |button| {
                    button.frame().y
                })
                - GAP * 2.0;
            items.push(Item::Text {
                origin: Point::new(
                    (centre - MESSAGE_WIDTH / 2.0).round(),
                    (bottom - note.height()).round(),
                ),
                paragraph: note.clone(),
            });
        }
        let (shown, _) = {
            let model = self.model.borrow();
            model.identity_shown.value_at(model.now)
        };
        if shown <= 0.0 {
            return;
        }
        let mut identity = Vec::new();
        self.paint_identity(cx, &mut identity);
        match shown >= 1.0 {
            true => items.extend(identity),
            false => items.push(Item::Opacity {
                alpha: shown,
                items: identity,
            }),
        }
    }

    fn paint_identity(&self, cx: &Context, items: &mut Vec<Item>) {
        let Some([_, _, initial, name]) = &self.paragraphs else {
            return;
        };
        let centre = self.frame.x + self.frame.width / 2.0;
        let avatar = self.avatar();
        if let Some(picture) = self.model.borrow().picture.clone() {
            items.push(Item::Clip {
                shape: Shape::rounded(avatar, AVATAR_RADIUS),
                items: vec![Item::Image {
                    rect: avatar,
                    image: picture,
                }],
            });
        } else {
            items.push(Item::Fill {
                shape: Shape::rounded(avatar, AVATAR_RADIUS),
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
        }
        items.push(Item::Text {
            origin: centred(name, centre, avatar.y + AVATAR + NAME_GAP),
            paragraph: name.clone(),
        });
        if let Some(users) = self.children.get(USERS_INDEX) {
            users.paint(cx, items);
        }
        self.paint_field_group(cx, items);
        for (child, label) in self.children.iter().skip(POWER_INDEX).zip(
            self.power_labels
                .iter()
                .map(Some)
                .chain(std::iter::repeat(None)),
        ) {
            child.paint(cx, items);
            if let Some(label) = label {
                let button = child.frame();
                items.push(Item::Text {
                    origin: centred(
                        label,
                        button.x + button.width / 2.0,
                        button.y + button.height + POWER_LABEL_GAP,
                    ),
                    paragraph: label.clone(),
                });
            }
        }
    }

    fn paint_field_group(&self, cx: &Context, items: &mut Vec<Item>) {
        let model = self.model.borrow();
        let checking = model.checking.is_some_and(|since| {
            model.now.saturating_duration_since(since) >= CHECKING_SHOWN_AFTER
        });
        drop(model);
        let group = items;
        if let Some(field) = self.children.get(FIELD_INDEX) {
            field.paint(cx, group);
            let centre = field.frame().x + field.frame().width / 2.0;
            let below = field.frame().y + field.frame().height + GAP;
            match (checking, &self.message) {
                (true, _) => {
                    if let Some(progress) = self.children.get(PROGRESS_INDEX) {
                        progress.paint(cx, group);
                    }
                }
                (false, Some(message)) => group.push(Item::Text {
                    origin: Point::new((centre - MESSAGE_WIDTH / 2.0).round(), below.round()),
                    paragraph: message.clone(),
                }),
                (false, None) => {}
            }
        }
    }

    fn paint_status(&self, cx: &Context, items: &mut Vec<Item>) {
        let mut held = self.model.borrow_mut();
        let model = &mut *held;
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
        if let Some((symbol, _)) = model.network {
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
        drop(held);
        if let Some(source) = self.children.get(SOURCE_INDEX) {
            source.paint(cx, items);
        }
    }

    fn status_left(&self) -> f32 {
        let model = self.model.borrow();
        let mut right = self.frame.x + self.frame.width - STATUS_MARGIN;
        if model.battery.is_some() {
            right -= BATTERY_SLOT + STATUS_GAP;
            if let Some(percent) = &self.percent {
                right -= PERCENT_GAP + percent.width();
            }
        }
        if let Some((symbol, _)) = model.network {
            right -= natural_size(symbol).0 + STATUS_GAP;
        }
        right
    }

    fn label_room(&self) -> f32 {
        self.power_labels
            .first()
            .map_or(0.0, |label| label.height() + POWER_LABEL_GAP)
    }

    fn power_top(&self, frame: Rect) -> f32 {
        let height = match self.power_labels.is_empty() {
            false => POWER_BUTTON,
            true => self.row.iter().map(|size| size.height).fold(0.0, f32::max),
        };
        let bottom = frame.y + frame.height - POWER_BOTTOM;
        match self.row.is_empty() {
            true => bottom,
            false => bottom - self.label_room() - height,
        }
    }

    fn avatar(&self) -> Rect {
        Rect::new(
            self.frame.x + ((self.frame.width - AVATAR) / 2.0).round(),
            self.frame.y + (self.frame.height * IDENTITY_TOP).round(),
            AVATAR,
            AVATAR,
        )
    }

    fn name_bottom(&self) -> f32 {
        let name = self
            .paragraphs
            .as_ref()
            .map_or(0.0, |[_, _, _, name]| name.height());
        self.avatar().y + AVATAR + NAME_GAP + name
    }
}

pub fn is_deliberate(event: &Event) -> bool {
    matches!(
        event,
        Event::PointerDown(_)
            | Event::PointerMove(_)
            | Event::Scroll(_)
            | Event::Key { .. }
            | Event::Commit(_)
    )
}

fn types_into_the_field(event: &Event) -> bool {
    match event {
        Event::Key {
            key: Key::Character(character),
            modifiers,
        } => !modifiers.control && !modifiers.alt && !modifiers.logo && !character.is_control(),
        Event::Commit(_) => true,
        _ => false,
    }
}

pub struct LockScreen {
    model: Shared,
    ui: Ui,
    view: WidgetId,
    field: WidgetId,
    source: WidgetId,
    users: WidgetId,
    chosen: Rc<Cell<Option<usize>>>,
    next_source: Rc<Cell<bool>>,
    formats: DateFormats,
}

fn network_of(wifi: Option<&Wifi>, wired: Option<bool>) -> Option<(Symbol, String)> {
    if let Some(network) = wifi.and_then(Wifi::current) {
        let label = tr!("Connected to {network}", network = network.name().as_str());
        return Some((wifi_symbol(wifi), label));
    }
    if wired == Some(true) {
        return Some((Symbol::Wired { connected: true }, tr!("Connected by cable")));
    }
    match (wifi, wired) {
        (Some(_), _) => Some((wifi_symbol(wifi), tr!("Offline"))),
        (None, Some(_)) => Some((Symbol::Wired { connected: false }, tr!("Offline"))),
        (None, None) => None,
    }
}

impl LockScreen {
    pub fn new(
        user: &User,
        formats: DateFormats,
        now: &jiff::Zoned,
        appearance: Appearance,
        text: TextSystem,
        purpose: Purpose,
    ) -> Self {
        let (date, time) = clock_texts(&formats, now);
        let model: Shared = Rc::new(RefCell::new(Model {
            purpose,
            input_source: None,
            question: None,
            messages: Messages::default(),
            backoff: Backoff::default(),
            cancel: false,
            note: String::new(),
            position: (0, 1),
            name: user.full_name.clone(),
            initial: user
                .full_name
                .chars()
                .next()
                .map(|initial| initial.to_uppercase().collect())
                .unwrap_or_default(),
            picture: None,
            date,
            time,
            submitted: None,
            checking: None,
            shake_from: None,
            now: Instant::now(),
            backdrop: None,
            backdrop_shown: Animated::new(0.0),
            battery: None,
            network: None,
            symbols: Symbols::default(),
            scale: 1.0,
            powers: Vec::new(),
            asking: None,
            power_press: None,
            wallpaper: None,
            content_shown: Animated::new(1.0),
            identity_shown: Animated::new(1.0),
            resting: false,
            rests_at: None,
            power_labels: Vec::new(),
        }));
        let submitted = model.clone();
        let field = SecureField::new(&tr!("Enter Password"), FIELD_FONT, move |answer| {
            let mut model = submitted.borrow_mut();
            let now = model.now;
            if model.checking.is_none()
                && !answer.is_empty()
                && model.backoff.waiting(now).is_none()
            {
                model.checking = Some(now);
                model.messages.clear();
                model.submitted = Some(Zeroizing::new(answer.to_owned()));
            }
        });
        let field = field.with_go_button();
        let field_id = field.id();
        let next_source = Rc::new(Cell::new(false));
        let source = SourceButton::new(next_source.clone());
        let source_id = source.id();
        let users = UserList::new(tr!("Users"));
        let users_id = users.id();
        let view_id = WidgetId::next();
        let view = LockView {
            id: view_id,
            model: model.clone(),
            children: vec![
                Box::new(field),
                Box::new(ProgressBar::new(&tr!("Checking"), None)),
                Box::new(source),
                Box::new(users),
            ],
            paragraphs: None,
            message: None,
            note: None,
            percent: None,
            power_labels: Vec::new(),
            row: Vec::new(),
            extras: [Size::default(); 2],
            frame: Rect::default(),
        };
        let ui = Ui::new(Box::new(view), Context::new(text, night(appearance)));
        let mut screen = Self {
            model,
            ui,
            view: view_id,
            field: field_id,
            source: source_id,
            users: users_id,
            chosen: Rc::new(Cell::new(None)),
            next_source,
            formats,
        };
        screen.set_appearance(appearance);
        screen.focus(field_id);
        screen
    }

    pub fn offer_power(&mut self, powers: &[PowerAction]) {
        self.model.borrow_mut().powers = powers.to_vec();
        self.show_power_row();
    }

    fn power_button(&self, title: &str, press: PowerPress) -> Button {
        let model = self.model.clone();
        Button::new(title, move || model.borrow_mut().power_press = Some(press))
    }

    fn power_symbol(&self, power: PowerAction, title: &str, color: Color) -> Button {
        let model = self.model.clone();
        let press = move || model.borrow_mut().power_press = Some(PowerPress::Ask(power));
        let icon = match power {
            PowerAction::Sleep => "system-suspend-symbolic",
            PowerAction::Restart => "system-reboot-symbolic",
            PowerAction::ShutDown => "system-shutdown-symbolic",
        };
        Button::symbol(
            find_icon(icon, POWER_ICON_PIXELS, &["Bande"], &data_dirs())
                .and_then(|path| Image::load_symbolic(&path, POWER_ICON_PIXELS, color)),
            title,
            press,
        )
    }

    fn show_power_row(&mut self) {
        let (powers, asking) = {
            let model = self.model.borrow();
            (model.powers.clone(), model.asking)
        };
        let mut titles = Vec::new();
        let row: Vec<Button> = match asking.and_then(PowerAction::ending) {
            Some(ending) => vec![
                self.power_button(&tr!("Cancel"), PowerPress::Cancel)
                    .cancel(),
                self.power_button(&ending.button(), PowerPress::Confirm)
                    .primary(),
            ],
            None => {
                let ink = self.ui.context().palette.ink;
                powers
                    .into_iter()
                    .map(|power| {
                        let title = power.ending().map_or_else(|| tr!("Sleep"), Ending::button);
                        titles.push(title.clone());
                        self.power_symbol(power, &title, ink)
                    })
                    .collect()
            }
        };
        self.model.borrow_mut().power_labels = titles;
        let focus = asking.and_then(|_| row.last().map(Widget::id));
        if let Some(view) = self.ui.widget_as::<LockView>(self.view) {
            view.children.truncate(POWER_INDEX);
            view.children.extend(
                row.into_iter()
                    .map(|button| Box::new(button) as Box<dyn Widget>),
            );
        }
        self.focus(focus.unwrap_or(self.field));
    }

    fn focus(&mut self, target: WidgetId) {
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
        model.question = asking.and_then(PowerAction::ending).map(Ending::question);
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
        self.ui.set_appearance(Appearance {
            scheme: Scheme::Dark,
            ..appearance
        });
    }

    pub fn set_input_source(&mut self, name: Option<String>) -> bool {
        if let Some(button) = self.ui.widget_as::<SourceButton>(self.source) {
            button.set_name(name.clone());
        }
        let mut model = self.model.borrow_mut();
        let changed = model.input_source != name;
        model.input_source = name;
        changed
    }

    pub fn take_next_input_source(&mut self) -> bool {
        self.next_source.take()
    }

    pub fn set_peers(&mut self, peers: Vec<Peer>, selected: usize) {
        let chosen = self.chosen.clone();
        if let Some(users) = self.ui.widget_as::<UserList>(self.users) {
            users.show(peers, selected, &chosen);
        }
    }

    pub fn take_chosen_peer(&mut self) -> Option<usize> {
        self.chosen.take()
    }

    pub fn set_caps_lock(&mut self, on: bool) -> bool {
        let cx = self.ui.context();
        let changed = cx.caps_lock != on;
        cx.caps_lock = on;
        changed
    }

    pub fn set_devices(
        &mut self,
        battery: Option<Battery>,
        wifi: Option<&Wifi>,
        wired: Option<bool>,
    ) -> bool {
        let mut model = self.model.borrow_mut();
        let network = network_of(wifi, wired);
        let changed = model.battery != battery || model.network != network;
        model.battery = battery;
        model.network = network;
        changed
    }

    pub fn set_scale(&mut self, scale: f64) {
        self.model.borrow_mut().scale = scale;
    }

    pub fn set_position(&mut self, index: usize, count: usize) {
        self.model.borrow_mut().position = (index, count);
    }

    pub fn set_message(&mut self, message: Option<String>) {
        let mut model = self.model.borrow_mut();
        match message {
            Some(message) => model.messages.push(clip_message(&message)),
            None => model.messages.clear(),
        }
    }

    pub fn ask(&mut self, prompt: Option<&Prompt>) {
        let placeholder = prompt
            .map(|prompt| prompt.text.trim().trim_end_matches(':').to_owned())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| tr!("Enter Password"));
        let visible = prompt.is_some_and(|prompt| prompt.visible);
        if let Some(field) = self.ui.widget_as::<SecureField>(self.field) {
            field.set_placeholder(&placeholder);
            field.set_revealed(visible);
        }
        self.clear_field();
        self.model.borrow_mut().checking = None;
    }

    pub fn is_checking(&self) -> bool {
        self.model.borrow().checking.is_some()
    }

    pub fn can_switch_person(&mut self) -> bool {
        let typed = self
            .ui
            .widget_as::<SecureField>(self.field)
            .is_some_and(|field| !field.secret().is_empty());
        let model = self.model.borrow();
        model.checking.is_none() && model.asking.is_none() && !typed
    }

    pub fn take_cancel(&mut self) -> bool {
        std::mem::take(&mut self.model.borrow_mut().cancel)
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

    pub fn start_resting(&mut self) {
        let mut model = self.model.borrow_mut();
        model.resting = true;
        model.identity_shown = Animated::new(0.0);
    }

    pub fn is_resting(&self) -> bool {
        self.model.borrow().resting
    }

    pub fn deadline(&self) -> Option<Instant> {
        let model = self.model.borrow();
        let backoff = model.backoff.waiting(model.now).map(|left| {
            let into_second = Duration::from_nanos(u64::from(left.subsec_nanos()));
            model.now
                + match into_second.is_zero() {
                    true => BACKOFF_TICK,
                    false => into_second,
                }
        });
        model.rests_at.into_iter().chain(backoff).min()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let waiting = {
            let mut model = self.model.borrow_mut();
            model.now = model.now.max(now);
            model
                .backoff
                .until()
                .is_some_and(|until| until + BACKOFF_TICK > now)
        };
        self.rest_when_due(now) | waiting
    }

    fn rest_when_due(&mut self, now: Instant) -> bool {
        let mut model = self.model.borrow_mut();
        if model.rests_at.is_none_or(|at| now < at) {
            return false;
        }
        if model.checking.is_some() {
            model.rests_at = Some(now + RESTS_AFTER);
            return false;
        }
        model.rests_at = None;
        model.resting = true;
        model.question = None;
        model.messages.clear();
        model
            .identity_shown
            .animate_at(now, false, 0.0, Spring::FADE);
        drop(model);
        self.clear_field();
        true
    }

    pub fn wake(&mut self, now: Instant) {
        let mut model = self.model.borrow_mut();
        model.rests_at = Some(now + RESTS_AFTER);
        if std::mem::take(&mut model.resting) {
            model
                .identity_shown
                .animate_at(now, false, 1.0, Spring::FADE);
        }
    }

    pub fn wake_on(&mut self, event: &Event, now: Instant) -> bool {
        let resting = self.is_resting();
        self.wake(now);
        !resting || types_into_the_field(event)
    }

    pub fn handle(&mut self, event: &Event) -> bool {
        if let Event::Key {
            key: Key::Space,
            modifiers,
        } = event
            && modifiers.logo
        {
            self.next_source.set(true);
            return true;
        }
        let escape = matches!(
            event,
            Event::Key {
                key: Key::Escape,
                ..
            }
        );
        if self.is_checking() {
            if escape {
                let mut model = self.model.borrow_mut();
                model.checking = None;
                model.cancel = true;
                return true;
            }
            return false;
        }
        if escape {
            if self.model.borrow().asking.is_some() {
                self.ask_power(None);
                return true;
            }
            self.clear_field();
            let mut model = self.model.borrow_mut();
            model.messages.clear();
            model.cancel = true;
            return true;
        }
        self.ui.handle(event)
    }

    pub fn perform(&mut self, request: &ActionRequest) -> bool {
        if self.is_checking() {
            return false;
        }
        self.ui.perform(request)
    }

    pub fn accessibility(&self) -> TreeUpdate {
        self.ui.accessibility()
    }

    pub fn take_answer(&mut self) -> Option<Zeroizing<String>> {
        self.model.borrow_mut().submitted.take()
    }

    pub fn reject(&mut self, at: Instant) {
        self.clear_field();
        let mut model = self.model.borrow_mut();
        model.checking = None;
        model.shake_from = Some(at);
    }

    pub fn refuse(&mut self, refusal: Refusal, at: Instant) {
        self.reject(at);
        let mut model = self.model.borrow_mut();
        model.messages.push(refusal.message());
        if refusal.counts_as_attempt() {
            model.backoff.fail(at);
        }
    }

    pub fn backoff(&self) -> Backoff {
        self.model.borrow().backoff.clone()
    }

    pub fn set_backoff(&mut self, backoff: Backoff) {
        self.model.borrow_mut().backoff = backoff;
    }

    pub fn accepted(&mut self) {
        let mut model = self.model.borrow_mut();
        model.backoff.succeed();
        model.messages.clear();
    }

    pub fn set_backdrop(&mut self, image: Image, now: Instant) {
        let mut model = self.model.borrow_mut();
        model.backdrop = Some(image);
        model
            .backdrop_shown
            .animate_at(now, false, 1.0, Spring::FADE);
    }

    pub fn set_note(&mut self, note: &str) {
        note.trim().clone_into(&mut self.model.borrow_mut().note);
    }

    pub fn set_picture(&mut self, picture: Image) {
        self.model.borrow_mut().picture = Some(picture);
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
            || model.identity_shown.value_at(model.now).1
            || model.content_shown.value_at(model.now).1
            || model.checking.is_some()
            || model
                .shake_from
                .is_some_and(|from| model.now < from + SHAKE_DURATION)
            || self.ui.is_animating()
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
    use super::*;
    use bande_toolkit::ui::{Modifiers, Pointer};

    #[test]
    fn a_click_on_the_input_source_keeps_typing_in_the_password_field() {
        let user = User {
            uid: 1000,
            login: "ada".to_owned(),
            full_name: "Ada Lovelace".to_owned(),
        };
        let mut screen = LockScreen::new(
            &user,
            DateFormats::for_locale(&bande_datetime::posix_locale("en_GB.UTF-8")),
            &"2026-10-28T09:41:00[UTC]".parse().unwrap_or_default(),
            Appearance::default(),
            TextSystem::new(),
            Purpose::Lock,
        );
        screen.set_input_source(Some("Svenska".to_owned()));
        screen.wake(Instant::now());
        screen.scene(480, 360);
        let at = screen
            .ui
            .widget_ref::<SourceButton>(screen.source)
            .map(|button| button.frame())
            .map(|frame| Point::new(frame.x + frame.width / 2.0, frame.y + frame.height / 2.0))
            .unwrap_or_default();
        screen.handle(&Event::PointerDown(Pointer::at(at)));
        screen.handle(&Event::PointerUp(Pointer::at(at)));
        assert!(screen.take_next_input_source());
        for key in [Key::Character('a'), Key::Space, Key::Enter] {
            screen.handle(&Event::Key {
                key,
                modifiers: Modifiers::default(),
            });
        }
        assert_eq!(
            screen.take_answer().as_deref().map(String::as_str),
            Some("a ")
        );
    }
}
