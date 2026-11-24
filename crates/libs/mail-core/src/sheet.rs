use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::tr;

use crate::discover::Settings;
use crate::guide::Options;
use crate::net::{Security, Server};
use accesskit::{Live, Node, Role};
use bande_toolkit::ui::{
    Animated, Button, CheckState, Checkbox, Context, PopUpButton, SPINNER_SIZE, SecureField, Size,
    Spinner, Spring, TextField, Widget, WidgetId,
};
use bande_toolkit::{
    Color, ControlSize, Image, Item, Paragraph, Path, Point, Rect, Shape, TextAlign, TextRole,
    TextStyle,
};
use zeroize::Zeroizing;

const WIDTH: f32 = 600.0;
const PADDING: f32 = 24.0;
const ICON_SIZE: f32 = 48.0;
const ICON_GAP: f32 = 16.0;
const TITLE_GAP: f32 = 6.0;
const LABEL_WIDTH: f32 = 128.0;
const LABEL_GAP: f32 = 10.0;
const ROW_GAP: f32 = 10.0;
const ERROR_GAP: f32 = 4.0;
const LINE_GAP: f32 = 8.0;
const PORT_WIDTH: f32 = 64.0;
const SECURITY_WIDTH: f32 = 168.0;
const SECTION_GAP: f32 = 18.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 96.0;
const MARK_GAP: f32 = 8.0;
const FIELD_FONT: f32 = 13.0;
const SHAKE_NUDGE: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Email,
    User,
    Password,
    Incoming,
    IncomingPort,
    IncomingSecurity,
    Outgoing,
    OutgoingPort,
    OutgoingSecurity,
    Description,
    Notes,
}

impl Field {
    fn label(self) -> String {
        match self {
            Self::Name => tr!("Name:"),
            Self::Email => tr!("Email address:"),
            Self::User => tr!("User name:"),
            Self::Password => tr!("Password:"),
            Self::Incoming => tr!("Incoming (IMAP):"),
            Self::Outgoing => tr!("Outgoing (SMTP):"),
            Self::IncomingPort | Self::OutgoingPort => tr!("Port:"),
            Self::IncomingSecurity | Self::OutgoingSecurity => tr!("Security:"),
            Self::Description => tr!("Description:"),
            Self::Notes => String::new(),
        }
    }

    fn placeholder(self) -> String {
        match self {
            Self::Name => tr!("Ann Lee"),
            Self::Email => tr!("name@example.org"),
            Self::User => tr!("Same as the email address"),
            Self::Password => tr!("Required"),
            Self::Incoming => tr!("imap.example.org"),
            Self::Outgoing => tr!("smtp.example.org"),
            Self::IncomingPort | Self::OutgoingPort => tr!("Default"),
            Self::Description => tr!("Work, Home…"),
            Self::IncomingSecurity | Self::OutgoingSecurity | Self::Notes => String::new(),
        }
    }

    fn starts_server_row(self) -> bool {
        matches!(self, Self::Incoming | Self::Outgoing)
    }

    fn row_start(self) -> Self {
        match self {
            Self::IncomingPort | Self::IncomingSecurity => Self::Incoming,
            Self::OutgoingPort | Self::OutgoingSecurity => Self::Outgoing,
            other => other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Address,
    Checking,
    Confirm(String),
    Servers,
    Added(String),
    Password(String),
}

impl Mode {
    fn fields(&self, offer_notes: bool) -> Vec<Field> {
        match self {
            Self::Address => vec![Field::Name, Field::Email, Field::Password],
            Self::Checking | Self::Confirm(_) => Vec::new(),
            Self::Servers => vec![
                Field::Name,
                Field::Email,
                Field::User,
                Field::Password,
                Field::Incoming,
                Field::IncomingPort,
                Field::IncomingSecurity,
                Field::Outgoing,
                Field::OutgoingPort,
                Field::OutgoingSecurity,
            ],
            Self::Added(_) if offer_notes => vec![Field::Description, Field::Notes],
            Self::Added(_) => vec![Field::Description],
            Self::Password(_) => vec![Field::Password],
        }
    }

    fn other(&self, server_settings: bool) -> Option<String> {
        match self {
            Self::Address => Some(tr!("Set Up Manually…")),
            Self::Confirm(_) => Some(tr!("Change Settings…")),
            Self::Servers => Some(tr!("Back")),
            Self::Password(_) if server_settings => Some(tr!("Server Settings…")),
            Self::Password(_) | Self::Checking | Self::Added(_) => None,
        }
    }

    fn primary(&self) -> Option<String> {
        match self {
            Self::Address => Some(tr!("Continue")),
            Self::Confirm(_) | Self::Servers | Self::Password(_) => Some(tr!("Sign In")),
            Self::Added(_) => Some(tr!("Done")),
            Self::Checking => None,
        }
    }

    fn cancels(&self, inline: bool) -> bool {
        match self {
            Self::Added(_) => false,
            Self::Address => !inline,
            _ => true,
        }
    }

    fn texts(&self) -> (String, String) {
        match self {
            Self::Address => (
                tr!("Add a Mail Account"),
                tr!("Enter your name, email address and password."),
            ),
            Self::Checking => (
                tr!("Add a Mail Account"),
                tr!("Mail is checking the account. This can take a moment."),
            ),
            Self::Confirm(message) => (tr!("Sign In to These Servers?"), message.clone()),
            Self::Servers => (
                tr!("Mail Server Settings"),
                tr!("Enter the settings your mail provider gives for IMAP and SMTP, then sign in."),
            ),
            Self::Added(email) => (
                tr!("Account Added"),
                tr!("{email} is ready.", email = email.clone()),
            ),
            Self::Password(email) => (
                tr!("Enter Your Password"),
                tr!(
                    "The server did not accept the password for {email}.",
                    email = email.clone()
                ),
            ),
        }
    }

    fn footnote(&self) -> Option<String> {
        match self {
            Self::Address => Some(tr!(
                "Mail works with any provider that offers IMAP and SMTP."
            )),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Primary,
    Other,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Waiting,
    Running,
    Done,
}

#[derive(Default)]
pub struct SheetState {
    pub action: Option<Action>,
    pub password: Zeroizing<String>,
    pub values: Vec<(Field, String)>,
    pub lines: Vec<(String, Option<Progress>)>,
    pub problem: Option<(Field, String)>,
    pub status: Option<(String, bool)>,
    pub shake: bool,
    pub closed: bool,
}

fn default_port(field: Field, security: Security) -> u16 {
    match (field, security) {
        (Field::Incoming, Security::Tls) => 993,
        (Field::Incoming, _) => 143,
        (_, Security::Tls) => 465,
        _ => 587,
    }
}

impl SheetState {
    pub fn value(&self, field: Field) -> String {
        self.values
            .iter()
            .find(|(known, _)| *known == field)
            .map(|(_, value)| value.trim().to_owned())
            .unwrap_or_default()
    }

    fn set(&mut self, field: Field, value: &str) {
        match self.values.iter_mut().find(|(known, _)| *known == field) {
            Some((_, known)) => value.clone_into(known),
            None => self.values.push((field, value.to_owned())),
        }
    }

    pub fn notes(&self) -> bool {
        self.value(Field::Notes) != "off"
    }

    fn security(&self, field: Field) -> Security {
        Security::from_name(&self.value(field)).unwrap_or(Security::Tls)
    }

    pub fn fill(&mut self, settings: &Settings) {
        for (host, port, security, server) in [
            (
                Field::Incoming,
                Field::IncomingPort,
                Field::IncomingSecurity,
                &settings.imap,
            ),
            (
                Field::Outgoing,
                Field::OutgoingPort,
                Field::OutgoingSecurity,
                &settings.smtp,
            ),
        ] {
            self.set(host, &server.host);
            self.set(port, &server.port.to_string());
            self.set(security, server.security.name());
        }
        self.set(Field::User, &settings.user);
    }

    pub fn servers(&self) -> Result<Settings, (Field, String)> {
        let server = |host: Field, port: Field, security: Field| {
            let name = self.value(host).to_ascii_lowercase();
            if name.is_empty() {
                return Err((host, tr!("Enter the server name.")));
            }
            let security = self.security(security);
            let number = match self.value(port).as_str() {
                "" => default_port(host, security),
                number => number
                    .parse()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| (port, tr!("Enter a port number from 1 to 65535.")))?,
            };
            Ok(Server {
                host: name,
                port: number,
                security,
            })
        };
        let user = match self.value(Field::User) {
            user if user.is_empty() => self.value(Field::Email),
            user => user,
        };
        Ok(Settings {
            imap: server(
                Field::Incoming,
                Field::IncomingPort,
                Field::IncomingSecurity,
            )?,
            smtp: server(
                Field::Outgoing,
                Field::OutgoingPort,
                Field::OutgoingSecurity,
            )?,
            user,
        })
    }
}

pub type Shared = Rc<RefCell<SheetState>>;

struct Slot {
    child: usize,
    rect: Rect,
}

#[derive(Default)]
struct Laid {
    texts: Vec<(Point, Paragraph)>,
    marks: Vec<(Point, Progress)>,
    slots: Vec<Slot>,
}

pub struct AccountSheet {
    id: WidgetId,
    state: Shared,
    fields: Vec<Field>,
    children: Vec<Box<dyn Widget>>,
    spinner: usize,
    buttons: Vec<usize>,
    mode: Mode,
    inline: bool,
    icon: Option<Image>,
    other: bool,
    laid: Laid,
    spinning: bool,
    shake: Animated,
    size: Size,
    frame: Rect,
}

fn spoken_label(field: Field) -> String {
    field.label().trim_end_matches([':', ' ']).to_owned()
}

fn problem_at(state: &SheetState, field: Field) -> Option<String> {
    state
        .problem
        .as_ref()
        .filter(|(known, _)| *known == field)
        .map(|(_, problem)| problem.clone())
}

fn text_field(state: &Shared, field: Field) -> TextField {
    let shared = Rc::clone(state);
    let (value, problem) = {
        let state = state.borrow();
        (state.value(field), problem_at(&state, field))
    };
    TextField::new(&field.placeholder(), &value, move |text| {
        shared.borrow_mut().set(field, text);
    })
    .without_text_checking()
    .named(&spoken_label(field), problem.as_deref())
}

fn security_choice(state: &Shared, field: Field) -> PopUpButton {
    let chosen = Security::ALL
        .iter()
        .position(|security| *security == state.borrow().security(field))
        .unwrap_or_default();
    let titles = Security::ALL.map(Security::title);
    let shared = Rc::clone(state);
    PopUpButton::new(&tr!("Security"), &titles, chosen, move |index| {
        if let Some(security) = Security::ALL.get(index) {
            shared.borrow_mut().set(field, security.name());
        }
    })
}

fn notes_choice(state: &Shared) -> Checkbox {
    let checked = match state.borrow().notes() {
        true => CheckState::On,
        false => CheckState::Off,
    };
    let shared = Rc::clone(state);
    Checkbox::new(
        &tr!("Also use this account for Notes"),
        checked,
        move |on| {
            let value = if on { "on" } else { "off" };
            shared.borrow_mut().set(Field::Notes, value);
        },
    )
}

fn action_button(state: &Shared, title: &str, action: Action) -> Button {
    let state = Rc::clone(state);
    Button::new(title, move || state.borrow_mut().action = Some(action))
}

fn paint_mark(cx: &Context, at: Point, progress: Progress, items: &mut Vec<Item>) {
    let size = SPINNER_SIZE;
    let point = |x: f32, y: f32| Point::new(at.x + x * size, at.y + y * size);
    match progress {
        Progress::Waiting => items.push(Item::Stroke {
            shape: Shape::rounded(
                Rect::new(at.x + 2.0, at.y + 2.0, size - 4.0, size - 4.0),
                size / 2.0 - 2.0,
            ),
            width: 1.5,
            paint: cx.palette.ink3.into(),
        }),
        Progress::Done => items.push(Item::StrokePath {
            path: Path::through(&[point(0.18, 0.52), point(0.42, 0.76), point(0.84, 0.26)]),
            width: 2.0,
            paint: cx.palette.success.into(),
        }),
        Progress::Running => {}
    }
}

fn body(cx: &mut Context, text: &str, role: TextRole, color: Color, width: f32) -> Paragraph {
    cx.text.paragraph(
        text,
        TextStyle::role(role, color),
        Some(width),
        TextAlign::Start,
    )
}

impl AccountSheet {
    pub fn new(state: Shared, mode: Mode, options: &Options, inline: bool) -> Self {
        let shake = std::mem::take(&mut state.borrow_mut().shake);
        let fields = mode.fields(options.offer_notes);
        let other = mode.other(options.server_settings);
        let mut children: Vec<Box<dyn Widget>> = Vec::new();
        for field in &fields {
            let child: Box<dyn Widget> = match field {
                Field::Password => {
                    let shared = Rc::clone(&state);
                    let (kept, problem) = {
                        let state = state.borrow();
                        (state.password.clone(), problem_at(&state, *field))
                    };
                    Box::new(
                        SecureField::new(&field.placeholder(), FIELD_FONT, move |secret| {
                            let mut state = shared.borrow_mut();
                            state.password = Zeroizing::new(secret.to_owned());
                            state.action = Some(Action::Primary);
                        })
                        .with_secret(&kept)
                        .named(&spoken_label(*field), problem.as_deref()),
                    )
                }
                Field::IncomingSecurity | Field::OutgoingSecurity => {
                    Box::new(security_choice(&state, *field))
                }
                Field::Notes => Box::new(notes_choice(&state)),
                _ => Box::new(text_field(&state, *field)),
            };
            children.push(child);
        }
        let spinner = children.len();
        children.push(Box::new(Spinner::new(&tr!("Checking the account…"))));
        let mut buttons = Vec::new();
        if let Some(other) = &other {
            buttons.push(children.len());
            children.push(Box::new(action_button(&state, other, Action::Other)));
        }
        if mode.cancels(inline) {
            buttons.push(children.len());
            children.push(Box::new(
                action_button(&state, &tr!("Cancel"), Action::Cancel).cancel(),
            ));
        }
        if let Some(primary) = mode.primary() {
            buttons.push(children.len());
            children.push(Box::new(
                action_button(&state, &primary, Action::Primary).primary(),
            ));
        }
        Self {
            id: WidgetId::next(),
            state,
            fields,
            children,
            spinner,
            buttons,
            mode,
            inline,
            icon: Image::from_svg(options.icon, 96, 96),
            other: other.is_some(),
            laid: Laid::default(),
            spinning: false,
            shake: Animated::points(match shake {
                true => SHAKE_NUDGE,
                false => 0.0,
            }),
            size: Size::default(),
            frame: Rect::default(),
        }
    }

    pub fn field(&self, field: Field) -> Option<WidgetId> {
        self.fields
            .iter()
            .position(|known| *known == field)
            .map(|index| self.children[index].id())
    }

    fn rows(&self) -> Vec<Vec<usize>> {
        let mut rows = Vec::new();
        let mut index = 0;
        while index < self.fields.len() {
            let width = match self.fields[index].starts_server_row() {
                true => 3.min(self.fields.len() - index),
                false => 1,
            };
            rows.push((index..index + width).collect());
            index += width;
        }
        rows
    }
}

impl Widget for AccountSheet {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = WIDTH.min(available.width);
        let text_x = PADDING + ICON_SIZE + ICON_GAP;
        let text_width = width - text_x - PADDING;
        let field_x = PADDING + LABEL_WIDTH + LABEL_GAP;
        let field_width = width - PADDING - field_x;
        let control = cx.control_height(ControlSize::Regular);
        if self.shake.target() != 0.0 {
            cx.animate(&mut self.shake, 0.0, Spring::SHAKE);
        }
        let state = Rc::clone(&self.state);
        let state = state.borrow();
        let mut laid = Laid::default();

        let (title, message) = self.mode.texts();
        let title = body(cx, &title, TextRole::Heading, cx.palette.ink, text_width);
        let message = body(cx, &message, TextRole::Body, cx.palette.ink2, text_width);
        let header = (title.height() + TITLE_GAP + message.height()).max(ICON_SIZE);
        let message_y = PADDING + title.height() + TITLE_GAP;
        laid.texts.push((Point::new(text_x, PADDING), title));
        laid.texts.push((Point::new(text_x, message_y), message));
        let mut y = PADDING + header + SECTION_GAP;

        let sizes = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(field_width, control)))
            .collect::<Vec<_>>();
        let rows = self.rows();
        for row in &rows {
            let height = row
                .iter()
                .map(|index| sizes[*index].height)
                .fold(control, f32::max);
            let first = self.fields[row[0]];
            let label = cx.text.paragraph(
                &first.label(),
                TextStyle::role(TextRole::Body, cx.palette.ink2),
                Some(LABEL_WIDTH),
                TextAlign::End,
            );
            let label_y = y + ((control.min(height) - label.height()) / 2.0).round();
            laid.texts.push((Point::new(PADDING, label_y), label));
            match row.as_slice() {
                [host, port, security] => {
                    let host_width = field_width - PORT_WIDTH - SECURITY_WIDTH - 2.0 * LABEL_GAP;
                    let port_x = field_x + host_width + LABEL_GAP;
                    for (child, x, width) in [
                        (*host, field_x, host_width),
                        (*port, port_x, PORT_WIDTH),
                        (*security, port_x + PORT_WIDTH + LABEL_GAP, SECURITY_WIDTH),
                    ] {
                        laid.slots.push(Slot {
                            child,
                            rect: Rect::new(x, y, width, height),
                        });
                    }
                }
                _ => laid.slots.extend(row.iter().map(|child| Slot {
                    child: *child,
                    rect: Rect::new(field_x, y, field_width, height),
                })),
            }
            y += height;
            if let Some((_, problem)) = state
                .problem
                .as_ref()
                .filter(|(field, _)| field.row_start() == first)
            {
                let error = body(
                    cx,
                    problem,
                    TextRole::Callout,
                    cx.palette.danger_text,
                    field_width,
                );
                let below = ERROR_GAP + error.height();
                laid.texts.push((Point::new(field_x, y + ERROR_GAP), error));
                y += below;
            }
            y += ROW_GAP;
        }
        if !rows.is_empty() {
            y += SECTION_GAP - ROW_GAP;
        }

        let (lines_x, lines_width) = match rows.is_empty() {
            true => (text_x, text_width),
            false => (field_x, field_width),
        };
        self.spinning = false;
        for (text, progress) in &state.lines {
            let indent = match progress {
                Some(_) => SPINNER_SIZE + MARK_GAP,
                None => 0.0,
            };
            let color = match progress {
                Some(Progress::Waiting) => cx.palette.ink3,
                _ => cx.palette.ink,
            };
            let line = body(cx, text, TextRole::Body, color, lines_width - indent);
            let height = line.height();
            let mark_y = y + ((height - SPINNER_SIZE) / 2.0).max(0.0).round();
            match progress {
                Some(Progress::Running) => {
                    self.spinning = true;
                    laid.slots.push(Slot {
                        child: self.spinner,
                        rect: Rect::new(lines_x, mark_y, SPINNER_SIZE, SPINNER_SIZE),
                    });
                }
                Some(progress) => laid.marks.push((Point::new(lines_x, mark_y), *progress)),
                None => {}
            }
            laid.texts.push((Point::new(lines_x + indent, y), line));
            y += height + LINE_GAP;
        }
        if !state.lines.is_empty() {
            y += SECTION_GAP - LINE_GAP;
        }

        let notes = self
            .mode
            .footnote()
            .map(|text| (text, false))
            .into_iter()
            .chain(state.status.clone());
        for (text, error) in notes {
            let color = match error {
                true => cx.palette.danger_text,
                false => cx.palette.ink2,
            };
            let note = body(cx, &text, TextRole::Callout, color, lines_width);
            let height = note.height();
            laid.texts.push((Point::new(lines_x, y), note));
            y += height + LINE_GAP;
        }
        y += SECTION_GAP - LINE_GAP;

        let mut right = width - PADDING;
        let has_other = self.other;
        for (order, index) in self.buttons.iter().enumerate().rev() {
            let size = Size::new(sizes[*index].width.max(BUTTON_MIN_WIDTH), control);
            let x = match order == 0 && has_other {
                true => PADDING,
                false => {
                    right -= size.width;
                    let x = right;
                    right -= BUTTON_SPACING;
                    x
                }
            };
            laid.slots.push(Slot {
                child: *index,
                rect: Rect::new(x, y, size.width, size.height),
            });
        }
        y += control + PADDING;
        self.laid = laid;
        self.size = Size::new(width, y.ceil());
        match self.inline {
            true => available,
            false => self.size,
        }
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = match self.inline {
            true => Rect::new(
                (frame.x + (frame.width - self.size.width) / 2.0).round(),
                (frame.y + ((frame.height - self.size.height) / 3.0).max(0.0)).round(),
                self.size.width,
                self.size.height,
            ),
            false => frame,
        };
        let (x, y) = (self.frame.x, self.frame.y);
        let shaken = self
            .state
            .borrow()
            .problem
            .as_ref()
            .map(|(field, _)| *field);
        let nudge = cx.sample(&self.shake);
        for index in 0..self.children.len() {
            let rect = match self.laid.slots.iter().find(|slot| slot.child == index) {
                Some(slot) => {
                    let moved = match shaken.is_some() && self.fields.get(index) == shaken.as_ref()
                    {
                        true => nudge,
                        false => 0.0,
                    };
                    Rect::new(
                        x + slot.rect.x + moved,
                        y + slot.rect.y,
                        slot.rect.width,
                        slot.rect.height,
                    )
                }
                None => Rect::default(),
            };
            self.children[index].place(cx, rect);
        }
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        let (x, y) = (self.frame.x, self.frame.y);
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: Rect::new(x + PADDING, y + PADDING, ICON_SIZE, ICON_SIZE),
                image: icon.clone(),
            });
        }
        for (at, paragraph) in &self.laid.texts {
            items.push(Item::Text {
                origin: Point::new(x + at.x, y + at.y),
                paragraph: paragraph.clone(),
            });
        }
        for (at, progress) in &self.laid.marks {
            paint_mark(cx, Point::new(x + at.x, y + at.y), *progress, items);
        }
        for (index, child) in self.children.iter().enumerate() {
            if index != self.spinner || self.spinning {
                child.paint(cx, items);
            }
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let (title, message) = self.mode.texts();
        let mut node = match self.inline {
            true => Node::new(Role::Group),
            false => {
                let mut node = Node::new(Role::Dialog);
                node.set_modal();
                node
            }
        };
        node.set_label(title);
        node.set_live(Live::Polite);
        let state = self.state.borrow();
        let details = std::iter::once(message)
            .chain(state.lines.iter().map(|(line, _)| line.clone()))
            .chain(state.problem.iter().map(|(_, problem)| problem.clone()))
            .chain(state.status.iter().map(|(status, _)| status.clone()))
            .collect::<Vec<_>>();
        node.set_description(details.join("\n"));
        node
    }

    fn dismissed(&self) -> bool {
        self.state.borrow().closed
    }

    fn shows_child(&self, index: usize) -> bool {
        index != self.spinner || self.spinning
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
