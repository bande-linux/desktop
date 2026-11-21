use std::cell::RefCell;
use std::rc::Rc;

use bande_l10n::tr;

use crate::discover::Settings;
use crate::net::{Security, Server};
use accesskit::{Node, Role};
use bande_toolkit::ui::{
    Button, Context, PopUpButton, RadioGroup, SPINNER_SIZE, SecureField, Size, Spinner, TextField,
    Widget, WidgetId,
};
use bande_toolkit::{
    ControlSize, Image, Item, Paragraph, Point, Rect, Shape, TextAlign, TextRole, TextStyle,
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
const PORT_WIDTH: f32 = 64.0;
const SECURITY_WIDTH: f32 = 168.0;
const SECTION_GAP: f32 = 18.0;
const BUTTON_SPACING: f32 = 8.0;
const BUTTON_MIN_WIDTH: f32 = 96.0;
const STATUS_GAP: f32 = 8.0;
const FIELD_FONT: f32 = 13.0;
const ICON: &[u8] = include_bytes!("../../data/org.bande.Mail.svg");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Kind,
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
}

impl Field {
    fn label(self) -> String {
        match self {
            Self::Kind => tr!("Account Type:"),
            Self::Name => tr!("Name:"),
            Self::Email => tr!("Email Address:"),
            Self::User => tr!("User Name:"),
            Self::Password => tr!("Password:"),
            Self::Incoming => tr!("Incoming (IMAP):"),
            Self::Outgoing => tr!("Outgoing (SMTP):"),
            Self::IncomingPort | Self::OutgoingPort => tr!("Port:"),
            Self::IncomingSecurity | Self::OutgoingSecurity => tr!("Security:"),
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
            Self::Kind | Self::IncomingSecurity | Self::OutgoingSecurity => String::new(),
        }
    }

    fn starts_server_row(self) -> bool {
        matches!(self, Self::Incoming | Self::Outgoing)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Kind,
    Address,
    Servers,
    Password(String),
}

impl Mode {
    fn fields(&self) -> Vec<Field> {
        match self {
            Self::Kind => vec![Field::Kind],
            Self::Address => vec![Field::Name, Field::Email, Field::Password],
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
            Self::Password(_) => vec![Field::Password],
        }
    }

    fn other(&self) -> Option<String> {
        match self {
            Self::Kind => None,
            Self::Address => Some(tr!("Enter Server Settings Manually")),
            Self::Servers => Some(tr!("Back")),
            Self::Password(_) => Some(tr!("Server Settings…")),
        }
    }

    fn primary(&self) -> String {
        match self {
            Self::Kind | Self::Address => tr!("Continue"),
            Self::Servers | Self::Password(_) => tr!("Sign In"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Primary,
    Other,
    Cancel,
}

#[derive(Default)]
pub struct SheetState {
    pub action: Option<Action>,
    pub password: Zeroizing<String>,
    pub values: Vec<(Field, String)>,
    pub status: Option<(String, bool)>,
    pub busy: bool,
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

    pub fn servers(&self) -> Option<Settings> {
        let server = |host: Field, port: Field, security: Field| {
            let name = self.value(host).to_ascii_lowercase();
            let security = self.security(security);
            let port = match self.value(port) {
                port if port.is_empty() => default_port(host, security),
                port => port.parse().ok()?,
            };
            (!name.is_empty()).then_some(Server {
                host: name,
                port,
                security,
            })
        };
        let user = match self.value(Field::User) {
            user if user.is_empty() => self.value(Field::Email),
            user => user,
        };
        Some(Settings {
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

pub struct AccountSheet {
    id: WidgetId,
    state: Shared,
    fields: Vec<Field>,
    children: Vec<Box<dyn Widget>>,
    password_field: WidgetId,
    mode: Mode,
    icon: Option<Image>,
    title: Option<Paragraph>,
    message: Option<Paragraph>,
    labels: Vec<Paragraph>,
    status: Option<Paragraph>,
    row_heights: Vec<f32>,
    button_sizes: Vec<Size>,
    frame: Rect,
}

fn text_field(state: &Shared, field: Field) -> TextField {
    let shared = Rc::clone(state);
    let value = state.borrow().value(field);
    TextField::new(&field.placeholder(), &value, move |text| {
        shared.borrow_mut().set(field, text);
    })
    .without_text_checking()
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

fn action_button(state: &Shared, title: &str, action: Action) -> Button {
    let state = Rc::clone(state);
    Button::new(title, move || state.borrow_mut().action = Some(action))
}

impl AccountSheet {
    pub fn new(state: Shared, mode: Mode) -> Self {
        let fields = mode.fields();
        let mut children: Vec<Box<dyn Widget>> = Vec::new();
        let mut password_field = WidgetId::next();
        for field in &fields {
            match field {
                Field::Kind => children.push(Box::new(RadioGroup::new(
                    &tr!("Account Type"),
                    &[tr!("IMAP and SMTP")],
                    Some(0),
                    |_| {},
                ))),
                Field::Password => {
                    let shared = Rc::clone(&state);
                    let secure =
                        SecureField::new(&field.placeholder(), FIELD_FONT, move |secret| {
                            let mut state = shared.borrow_mut();
                            if !secret.is_empty() {
                                state.password = Zeroizing::new(secret.to_owned());
                            }
                            state.action = Some(Action::Primary);
                        });
                    password_field = secure.id();
                    children.push(Box::new(secure));
                }
                Field::IncomingSecurity | Field::OutgoingSecurity => {
                    children.push(Box::new(security_choice(&state, *field)));
                }
                _ => children.push(Box::new(text_field(&state, *field))),
            }
        }
        children.push(Box::new(
            action_button(&state, &tr!("Cancel"), Action::Cancel).cancel(),
        ));
        children.push(Box::new(
            action_button(&state, &mode.primary(), Action::Primary).primary(),
        ));
        children.push(Box::new(Spinner::new(&tr!("Checking the account…"))));
        if let Some(other) = mode.other() {
            children.push(Box::new(action_button(&state, &other, Action::Other)));
        }
        Self {
            id: WidgetId::next(),
            state,
            fields,
            children,
            password_field,
            mode,
            icon: Image::from_svg(ICON, 96, 96),
            title: None,
            message: None,
            labels: Vec::new(),
            status: None,
            row_heights: Vec::new(),
            button_sizes: Vec::new(),
            frame: Rect::default(),
        }
    }

    pub fn password_field(&self) -> WidgetId {
        self.password_field
    }

    fn text_x(&self) -> f32 {
        self.frame.x + PADDING + ICON_SIZE + ICON_GAP
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

    fn header_height(&self) -> f32 {
        let text = self.title.as_ref().map_or(0.0, Paragraph::height)
            + self
                .message
                .as_ref()
                .map_or(0.0, |message| TITLE_GAP + message.height());
        text.max(ICON_SIZE)
    }

    fn fields_height(&self) -> f32 {
        let rows = self.row_heights.len() as f32;
        self.row_heights.iter().sum::<f32>() + (rows - 1.0).max(0.0) * ROW_GAP
    }

    fn texts(&self) -> (String, String) {
        match &self.mode {
            Mode::Kind => (
                tr!("Add a Mail Account"),
                tr!(
                    "Choose the type of account. Mail reads and sends mail with IMAP and SMTP, the open standards mail providers offer."
                ),
            ),
            Mode::Address => (
                tr!("Add a Mail Account"),
                tr!(
                    "Enter your name, email address and password. Mail looks up the servers of your address’s domain and shows them to you before it signs in."
                ),
            ),
            Mode::Servers => (
                tr!("Mail Server Settings"),
                tr!("Check the settings your mail provider gives for IMAP and SMTP, then sign in."),
            ),
            Mode::Password(email) => (
                tr!("Enter Your Password"),
                tr!(
                    "The server did not accept the password for {email}.",
                    email = email.clone()
                ),
            ),
        }
    }
}

impl Widget for AccountSheet {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn measure(&mut self, cx: &mut Context, available: Size) -> Size {
        let width = WIDTH.min(available.width);
        let text_width = width - (PADDING + ICON_SIZE + ICON_GAP) - PADDING;
        let (title, message) = self.texts();
        self.title = Some(cx.text.paragraph(
            &title,
            TextStyle::role(TextRole::Heading, cx.palette.ink),
            Some(text_width),
            TextAlign::Start,
        ));
        self.message = Some(cx.text.paragraph(
            &message,
            TextStyle::role(TextRole::Body, cx.palette.ink2),
            Some(text_width),
            TextAlign::Start,
        ));
        self.labels = self
            .fields
            .iter()
            .map(|field| {
                cx.text.paragraph(
                    &field.label(),
                    TextStyle::role(TextRole::Body, cx.palette.ink2),
                    Some(LABEL_WIDTH),
                    TextAlign::End,
                )
            })
            .collect();
        let (status, error) = self
            .state
            .borrow()
            .status
            .clone()
            .map_or((None, false), |(text, error)| (Some(text), error));
        let color = match error {
            true => cx.palette.danger_text,
            false => cx.palette.ink2,
        };
        self.status = status.map(|status| {
            cx.text.paragraph(
                &status,
                TextStyle::role(TextRole::Callout, color),
                Some(width - 2.0 * PADDING - SPINNER_SIZE - STATUS_GAP),
                TextAlign::Start,
            )
        });
        let control = cx.control_height(ControlSize::Regular);
        let fields = self.fields.len();
        let sizes = self
            .children
            .iter_mut()
            .map(|child| child.measure(cx, Size::new(width, control)))
            .collect::<Vec<_>>();
        self.row_heights = self
            .rows()
            .iter()
            .map(|row| {
                row.iter()
                    .map(|index| sizes[*index].height)
                    .fold(control, f32::max)
            })
            .collect();
        self.button_sizes = sizes[fields..]
            .iter()
            .map(|size| Size::new(size.width.max(BUTTON_MIN_WIDTH), control))
            .collect();
        let status = self
            .status
            .as_ref()
            .map_or(SPINNER_SIZE, |status| status.height().max(SPINNER_SIZE));
        let height = PADDING
            + self.header_height()
            + SECTION_GAP
            + self.fields_height()
            + SECTION_GAP
            + status
            + SECTION_GAP
            + control
            + PADDING;
        Size::new(width, height.ceil())
    }

    fn place(&mut self, cx: &mut Context, frame: Rect) {
        self.frame = frame;
        let field_x = frame.x + PADDING + LABEL_WIDTH + LABEL_GAP;
        let field_width = frame.x + frame.width - PADDING - field_x;
        let mut y = frame.y + PADDING + self.header_height() + SECTION_GAP;
        for (row, height) in self.rows().into_iter().zip(self.row_heights.clone()) {
            match row.as_slice() {
                [host, port, security] => {
                    let host_width = field_width - PORT_WIDTH - SECURITY_WIDTH - 2.0 * LABEL_GAP;
                    self.children[*host].place(cx, Rect::new(field_x, y, host_width, height));
                    let port_x = field_x + host_width + LABEL_GAP;
                    self.children[*port].place(cx, Rect::new(port_x, y, PORT_WIDTH, height));
                    let security_x = port_x + PORT_WIDTH + LABEL_GAP;
                    self.children[*security]
                        .place(cx, Rect::new(security_x, y, SECURITY_WIDTH, height));
                }
                [single] => {
                    self.children[*single].place(cx, Rect::new(field_x, y, field_width, height));
                }
                _ => {}
            }
            y += height + ROW_GAP;
        }
        let fields = self.fields.len();
        let control = cx.control_height(ControlSize::Regular);
        let buttons_y = frame.y + frame.height - PADDING - control;
        let right = frame.x + frame.width - PADDING;
        let [cancel, primary] = [self.button_sizes[0], self.button_sizes[1]];
        let primary_x = right - primary.width;
        let cancel_x = primary_x - BUTTON_SPACING - cancel.width;
        self.children[fields + 1].place(
            cx,
            Rect::new(primary_x, buttons_y, primary.width, primary.height),
        );
        self.children[fields].place(
            cx,
            Rect::new(cancel_x, buttons_y, cancel.width, cancel.height),
        );
        if let Some(other) = self.button_sizes.get(3).copied() {
            self.children[fields + 3].place(
                cx,
                Rect::new(frame.x + PADDING, buttons_y, other.width, other.height),
            );
        }
        let status_y = buttons_y - SECTION_GAP - SPINNER_SIZE;
        self.children[fields + 2].place(
            cx,
            Rect::new(frame.x + PADDING, status_y, SPINNER_SIZE, SPINNER_SIZE),
        );
    }

    fn frame(&self) -> Rect {
        self.frame
    }

    fn paint(&self, cx: &Context, items: &mut Vec<Item>) {
        if let Some(icon) = &self.icon {
            items.push(Item::Image {
                rect: Rect::new(
                    self.frame.x + PADDING,
                    self.frame.y + PADDING,
                    ICON_SIZE,
                    ICON_SIZE,
                ),
                image: icon.clone(),
            });
        }
        let x = self.text_x();
        let mut y = self.frame.y + PADDING;
        if let Some(title) = &self.title {
            items.push(Item::Text {
                origin: Point::new(x, y),
                paragraph: title.clone(),
            });
            y += title.height() + TITLE_GAP;
        }
        if let Some(message) = &self.message {
            items.push(Item::Text {
                origin: Point::new(x, y),
                paragraph: message.clone(),
            });
        }
        let control = cx.control_height(ControlSize::Regular);
        let mut y = self.frame.y + PADDING + self.header_height() + SECTION_GAP;
        for (row, height) in self.rows().iter().zip(&self.row_heights) {
            if let Some(label) = row.first().and_then(|index| self.labels.get(*index)) {
                items.push(Item::Text {
                    origin: Point::new(
                        self.frame.x + PADDING,
                        y + ((control.min(*height) - label.height()) / 2.0).round(),
                    ),
                    paragraph: label.clone(),
                });
            }
            y += height + ROW_GAP;
        }
        let fields = self.fields.len();
        let busy = self.state.borrow().busy;
        for (index, child) in self.children.iter().enumerate() {
            if index == fields + 2 && !busy {
                continue;
            }
            child.paint(cx, items);
        }
        if let Some(status) = &self.status {
            let spinner = self.children[fields + 2].frame();
            let x = match busy {
                true => spinner.x + SPINNER_SIZE + STATUS_GAP,
                false => spinner.x,
            };
            items.push(Item::Clip {
                shape: Shape::rect(Rect::new(
                    x,
                    spinner.y - status.height(),
                    self.frame.width,
                    status.height() * 3.0,
                )),
                items: vec![Item::Text {
                    origin: Point::new(
                        x,
                        spinner.y + ((SPINNER_SIZE - status.height()) / 2.0).round(),
                    ),
                    paragraph: status.clone(),
                }],
            });
        }
    }

    fn access(&self, _cx: &Context) -> Node {
        let mut node = Node::new(Role::Dialog);
        let (title, message) = self.texts();
        node.set_label(title);
        node.set_description(message);
        node.set_modal();
        node
    }

    fn dismissed(&self) -> bool {
        self.state.borrow().closed
    }

    fn children(&self) -> &[Box<dyn Widget>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Box<dyn Widget>] {
        &mut self.children
    }
}
