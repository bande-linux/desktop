use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use accesskit::{Action, ActionRequest, TreeId};
use bande_discovery::{Resolver, domain_of};
use bande_inbox::Outbox;
use bande_l10n::tr;
use bande_toolkit::ui::{
    Button, Event, Key, Label, Modifiers, RadioGroup, SecureField, TEXT_FIELD_HEIGHT, TextField,
    Ui, Widget, WidgetId,
};

use super::panel::{Line, Panel, Width};
use crate::caldav::{Client, Credentials, Discovered, Error};
use crate::ical::CalendarObject;
use crate::store::{Account, AccountKind};
use crate::worker::http_url;

const WIDTH: f32 = 500.0;
const LABEL_WIDTH: f32 = 104.0;
const ROW: f32 = TEXT_FIELD_HEIGHT;
const NOTE: f32 = 54.0;
const KINDS: f32 = 44.0;
const BUTTON: f32 = 96.0;
const WIDE_BUTTON: f32 = 232.0;

static NEXT_SETUP: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Kind,
    Address,
    Server,
    Subscription,
    Password,
}

#[derive(Debug, Clone, PartialEq)]
enum Signal {
    Kind(usize),
    Address(String),
    User(String),
    Server(String),
    Password(String),
    Entered,
    Primary,
    Other,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Add,
    Subscribe(String),
    Password {
        account: String,
        name: String,
        user: String,
        server: String,
    },
    Edit {
        account: String,
        name: String,
        user: String,
        server: String,
    },
}

#[derive(Debug)]
pub struct Reply {
    setup: u64,
    answer: Answer,
}

#[derive(Debug)]
enum Answer {
    Found(Option<String>),
    Signed(Result<Discovered, Error>),
    Fetched(Result<String, Error>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Done {
    Added {
        server: String,
        user: String,
        password: String,
        found: Discovered,
    },
    Updated {
        account: String,
        server: String,
        user: String,
        password: String,
        found: Discovered,
    },
    Subscribed {
        url: String,
        data: String,
    },
    Password {
        account: String,
        user: String,
        password: String,
    },
}

fn caldav(account: &Account) -> Option<(String, String, String, String)> {
    match &account.kind {
        AccountKind::CalDav { user, server, .. } => Some((
            account.id.clone(),
            account.name.clone(),
            user.clone(),
            server.clone(),
        )),
        _ => None,
    }
}

/// The password sheet for a CalDAV account whose server refused the saved password.
pub fn sign_in(account: &Account) -> Option<Start> {
    let (account, name, user, server) = caldav(account)?;
    Some(Start::Password {
        account,
        name,
        user,
        server,
    })
}

/// The server settings of a CalDAV account, to check and change.
pub fn edit(account: &Account) -> Option<Start> {
    let (account, name, user, server) = caldav(account)?;
    Some(Start::Edit {
        account,
        name,
        user,
        server,
    })
}

pub struct AccountSetup {
    id: u64,
    outbox: Outbox<Reply>,
    signals: Rc<RefCell<Vec<Signal>>>,
    step: Step,
    subscription: bool,
    account: Option<String>,
    name: String,
    address: String,
    user: String,
    server: String,
    url: String,
    password: Option<String>,
    password_field: Option<WidgetId>,
    busy: bool,
    note: WidgetId,
    panel: WidgetId,
    cancel: WidgetId,
    dismissed: Rc<Cell<bool>>,
    closed: bool,
}

fn sender(signals: &Rc<RefCell<Vec<Signal>>>) -> impl Fn(Signal) + 'static {
    let signals = Rc::clone(signals);
    move |signal| signals.borrow_mut().push(signal)
}

fn problem(error: &Error, subscription: bool) -> String {
    match error {
        Error::Network(_) => tr!(
            "Calendar could not reach the server. Check the address, and that this computer is online."
        ),
        Error::Certificate(server) => tr!(
            "The certificate of {server} could not be verified. Check that the address is exactly the one your provider gives.",
            server = server.clone()
        ),
        Error::Unauthorized if subscription => tr!(
            "This calendar needs a sign-in. Ask its publisher for a public address, or add it as a CalDAV account."
        ),
        Error::Unauthorized => tr!(
            "The server did not accept the user name or password. Check both; some providers need a separate app password for calendar apps."
        ),
        Error::Unsupported(schemes) => tr!(
            "The server asks for a sign-in method Calendar does not support ({schemes}). Ask your provider for sign-in with a password.",
            schemes = schemes.clone()
        ),
        Error::Insecure(server) => tr!(
            "{server} offers no encrypted connection, so Calendar does not send the password. Use an https:// address.",
            server = server.clone()
        ),
        Error::Status { status, .. } if *status >= 500 => tr!(
            "The server answered with an error ({status}). Try again later.",
            status = *status
        ),
        _ if subscription => tr!("There is no calendar at this address. Check the link."),
        _ => tr!(
            "There is no CalDAV calendar at this address. Check the server address and path your provider gives."
        ),
    }
}

impl AccountSetup {
    pub fn new(start: Start, outbox: Outbox<Reply>) -> Self {
        let mut setup = Self {
            id: NEXT_SETUP.fetch_add(1, Ordering::Relaxed),
            outbox,
            signals: Rc::default(),
            step: Step::Kind,
            subscription: false,
            account: None,
            name: String::new(),
            address: String::new(),
            user: String::new(),
            server: String::new(),
            url: String::new(),
            password: None,
            password_field: None,
            busy: false,
            note: WidgetId::next(),
            panel: WidgetId::next(),
            cancel: WidgetId::next(),
            dismissed: Rc::default(),
            closed: false,
        };
        match start {
            Start::Add => {}
            Start::Subscribe(url) => {
                setup.step = Step::Subscription;
                setup.subscription = true;
                setup.url = url;
            }
            Start::Password {
                account,
                name,
                user,
                server,
            } => {
                setup.step = Step::Password;
                (setup.account, setup.name, setup.user, setup.server) =
                    (Some(account), name, user, server);
            }
            Start::Edit {
                account,
                name,
                user,
                server,
            } => {
                setup.step = Step::Server;
                (setup.account, setup.name, setup.user, setup.server) =
                    (Some(account), name, user, server);
            }
        }
        setup
    }

    pub fn closed(&self) -> bool {
        self.closed
    }

    fn adding(&self) -> bool {
        self.account.is_none()
    }

    fn field(
        &self,
        panel: &mut Panel,
        label: &str,
        placeholder: &str,
        value: &str,
        make: fn(String) -> Signal,
    ) {
        let send = sender(&self.signals);
        let field = panel.add(
            TextField::new(placeholder, value, move |text| send(make(text.to_owned())))
                .without_text_checking(),
        );
        panel.line(Line::new(Some(label), vec![(field, Width::Fill)], ROW));
    }

    fn password_line(&mut self, panel: &mut Panel) {
        let (send, entered) = (sender(&self.signals), sender(&self.signals));
        let field = panel.add(SecureField::new("", 13.0, move |secret| {
            send(Signal::Password(secret.to_owned()));
            entered(Signal::Entered);
        }));
        panel.line(Line::new(
            Some(&tr!("Password:")),
            vec![(field, Width::Fill)],
            ROW,
        ));
        self.password_field = panel.id_of(field);
    }

    fn note_line(&mut self, panel: &mut Panel, text: &str) {
        let note = panel.add(Label::new(text));
        panel.line(Line::new(None, vec![(note, Width::Fill)], NOTE));
        self.note = panel.id_of(note).unwrap_or_else(WidgetId::next);
    }

    fn buttons(&mut self, panel: &mut Panel, other: Option<(String, f32)>, primary: &str) {
        let mut cells = Vec::new();
        if let Some((other, width)) = other {
            let send = sender(&self.signals);
            let button = panel.add(Button::new(&other, move || send(Signal::Other)));
            cells.push((button, Width::Fixed(width)));
        }
        let send = sender(&self.signals);
        let cancel = panel.add(Button::new(&tr!("Cancel"), move || send(Signal::Cancel)).cancel());
        let send = sender(&self.signals);
        let primary = panel.add(Button::new(primary, move || send(Signal::Primary)).primary());
        cells.extend([
            (cancel, Width::Fixed(BUTTON)),
            (primary, Width::Fixed(BUTTON)),
        ]);
        panel.line(Line::new(None, cells, ROW).trailing());
        self.cancel = panel.id_of(cancel).unwrap_or_else(WidgetId::next);
    }

    fn build(&mut self) -> Panel {
        self.password_field = None;
        let title = match (self.step, &self.account) {
            (Step::Kind, _) => tr!("Add a Calendar Account"),
            (Step::Address, _) => tr!("Add a CalDAV Account"),
            (Step::Server, None) => tr!("CalDAV Server Settings"),
            (Step::Server | Step::Password, Some(_)) => {
                tr!("Sign in to “{name}”", name = self.name.clone())
            }
            (Step::Subscription, _) => tr!("Subscribe to a Calendar"),
            (Step::Password, None) => tr!("Sign In"),
        };
        let mut panel = Panel::new(Some(&title), WIDTH, LABEL_WIDTH).sheet();
        match self.step {
            Step::Kind => {
                let send = sender(&self.signals);
                let kinds = [
                    tr!("CalDAV account"),
                    tr!("Calendar subscription (webcal or https link)"),
                ];
                let chosen = usize::from(self.subscription);
                let kind = panel.add(RadioGroup::new(
                    &tr!("Account type:"),
                    &kinds,
                    Some(chosen),
                    move |index| send(Signal::Kind(index)),
                ));
                panel.line(Line::new(
                    Some(&tr!("Account type:")),
                    vec![(kind, Width::Fill)],
                    KINDS,
                ));
                self.note_line(
                    &mut panel,
                    &tr!("A CalDAV account keeps calendars in sync with a server. A subscription shows a published calendar, read-only."),
                );
                self.buttons(&mut panel, None, &tr!("Continue"));
            }
            Step::Address => {
                self.field(
                    &mut panel,
                    &tr!("Address:"),
                    &tr!("jane@example.org"),
                    &self.address.clone(),
                    Signal::Address,
                );
                self.password_line(&mut panel);
                self.note_line(
                    &mut panel,
                    &tr!("Calendar looks up the CalDAV server of your address’s domain and shows it to you before it signs in."),
                );
                self.buttons(
                    &mut panel,
                    Some((tr!("Enter Server Settings Manually"), WIDE_BUTTON)),
                    &tr!("Continue"),
                );
            }
            Step::Server => {
                self.field(
                    &mut panel,
                    &tr!("Server:"),
                    &tr!("https://dav.example.org/"),
                    &self.server.clone(),
                    Signal::Server,
                );
                self.field(
                    &mut panel,
                    &tr!("User name:"),
                    &tr!("jane@example.org"),
                    &self.user.clone(),
                    Signal::User,
                );
                self.password_line(&mut panel);
                self.note_line(
                    &mut panel,
                    &tr!("Enter the server address your provider gives, with its port and path if it has them. Calendar sends the password only when the server asks for it."),
                );
                let back = self.adding().then(|| (tr!("Back"), BUTTON));
                self.buttons(&mut panel, back, &tr!("Sign In"));
            }
            Step::Subscription => {
                self.field(
                    &mut panel,
                    &tr!("Address:"),
                    &tr!("webcal://example.org/calendar.ics"),
                    &self.url.clone(),
                    Signal::Server,
                );
                self.note_line(
                    &mut panel,
                    &tr!("Paste a webcal:// or https:// link to a calendar. It is read-only and refreshes as set in Calendar Settings."),
                );
                self.buttons(&mut panel, Some((tr!("Back"), BUTTON)), &tr!("Subscribe"));
            }
            Step::Password => {
                self.password_line(&mut panel);
                self.note_line(
                    &mut panel,
                    &tr!(
                        "The server did not accept the password for {user}. Enter it again, or check the server settings.",
                        user = self.user.clone()
                    ),
                );
                self.buttons(
                    &mut panel,
                    Some((tr!("Server Settings…"), WIDE_BUTTON)),
                    &tr!("Sign In"),
                );
            }
        }
        self.panel = panel.id();
        self.dismissed = panel.dismisser();
        panel
    }

    pub fn present(&mut self, ui: &mut Ui) {
        self.dismissed.set(true);
        let panel = self.build();
        ui.present_sheet(panel);
        if let Some(field) = self.password_field.filter(|_| self.step == Step::Password) {
            ui.set_focus(field);
        }
    }

    fn show_note(&mut self, ui: &mut Ui, message: &str) {
        let replacement = Label::new(message);
        let id = replacement.id();
        if let Some(panel) = ui.widget_as::<Panel>(self.panel)
            && panel.replace(self.note, replacement)
        {
            self.note = id;
        }
    }

    fn fail(&mut self, ui: &mut Ui, message: &str) {
        self.busy = false;
        self.show_note(ui, message);
    }

    fn collect_password(&mut self, ui: &mut Ui) {
        let Some(field) = self.password_field else {
            return;
        };
        ui.set_focus(field);
        ui.handle(&Event::Key {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        let collected = std::mem::take(&mut *self.signals.borrow_mut());
        for signal in collected {
            match signal {
                Signal::Entered => {}
                other => self.apply(other),
            }
        }
    }

    fn apply(&mut self, signal: Signal) {
        match signal {
            Signal::Kind(index) => self.subscription = index == 1,
            Signal::Address(text) => self.address = text.trim().to_owned(),
            Signal::User(text) => self.user = text.trim().to_owned(),
            Signal::Server(text) if self.step == Step::Subscription => {
                self.url = text.trim().to_owned()
            }
            Signal::Server(text) => self.server = text.trim().to_owned(),
            Signal::Password(secret) => self.password = Some(secret),
            Signal::Entered | Signal::Primary | Signal::Other | Signal::Cancel => {}
        }
    }

    /// Handles what the user did in the sheet, after the window's events.
    pub fn update(&mut self, ui: &mut Ui) -> Option<Done> {
        loop {
            let signals = std::mem::take(&mut *self.signals.borrow_mut());
            if signals.is_empty() || self.closed {
                return None;
            }
            for signal in signals {
                let done = match signal {
                    Signal::Primary | Signal::Entered if !self.busy => self.submit(ui),
                    Signal::Other if !self.busy => {
                        self.other(ui);
                        None
                    }
                    Signal::Cancel => {
                        self.closed = true;
                        self.dismissed.set(true);
                        None
                    }
                    other => {
                        self.apply(other);
                        None
                    }
                };
                if done.is_some() {
                    return done;
                }
            }
        }
    }

    fn other(&mut self, ui: &mut Ui) {
        self.collect_password(ui);
        self.step = match self.step {
            Step::Address => {
                if self.user.is_empty() {
                    self.user = self.address.clone();
                }
                Step::Server
            }
            Step::Server => Step::Address,
            Step::Subscription => Step::Kind,
            Step::Password => Step::Server,
            Step::Kind => return,
        };
        self.present(ui);
    }

    fn spawn(&mut self, ui: &mut Ui, status: &str, work: impl FnOnce() -> Answer + Send + 'static) {
        self.busy = true;
        self.show_note(ui, status);
        let (outbox, setup) = (self.outbox.clone(), self.id);
        let spawned = std::thread::Builder::new()
            .name("account-setup".to_owned())
            .spawn(move || {
                outbox.send(Reply {
                    setup,
                    answer: work(),
                });
            });
        if let Err(error) = spawned {
            self.fail(ui, &error.to_string());
        }
    }

    fn submit(&mut self, ui: &mut Ui) -> Option<Done> {
        if self.step != Step::Kind && self.step != Step::Subscription {
            self.collect_password(ui);
        }
        let password = self.password.clone().unwrap_or_default();
        match self.step {
            Step::Kind => {
                self.step = match self.subscription {
                    true => Step::Subscription,
                    false => Step::Address,
                };
                self.present(ui);
            }
            Step::Address => {
                if domain_of(&self.address).is_none() {
                    self.fail(ui, &tr!("Enter your address, such as jane@example.org."));
                } else {
                    let address = self.address.clone();
                    self.spawn(ui, &tr!("Looking up the server…"), move || {
                        Answer::Found(Client::find(&address, &Resolver::system()))
                    });
                }
            }
            Step::Server if self.server.is_empty() => {
                self.fail(ui, &tr!("Enter the address of the server."));
            }
            Step::Server if self.user.is_empty() => {
                self.fail(ui, &tr!("Enter your user name."));
            }
            Step::Server if password.is_empty() => {
                self.fail(ui, &tr!("Enter your password."));
            }
            Step::Server => {
                let credentials = Credentials {
                    user: self.user.clone(),
                    password,
                };
                let server = self.server.clone();
                self.spawn(ui, &tr!("Signing in…"), move || {
                    Answer::Signed(Client::new(Some(&credentials)).discover(&server))
                });
            }
            Step::Subscription if self.url.is_empty() => {
                self.fail(ui, &tr!("Enter the address of the calendar."));
            }
            Step::Subscription => {
                let url = http_url(&self.url);
                self.spawn(ui, &tr!("Updating…"), move || {
                    Answer::Fetched(Client::new(None).get(&url).and_then(|resource| {
                        match CalendarObject::parse(&resource.data) {
                            Ok(_) => Ok(resource.data),
                            Err(_) => Err(Error::NotFound(url)),
                        }
                    }))
                });
            }
            Step::Password if password.is_empty() => {
                self.fail(ui, &tr!("Enter your password."));
            }
            Step::Password => {
                let account = self.account.clone()?;
                self.close(ui);
                return Some(Done::Password {
                    account,
                    user: self.user.clone(),
                    password,
                });
            }
        }
        None
    }

    /// Takes the answer of the lookup or sign-in that this setup started.
    pub fn receive(&mut self, ui: &mut Ui, reply: Reply) -> Option<Done> {
        if reply.setup != self.id || self.closed {
            return None;
        }
        self.busy = false;
        match reply.answer {
            Answer::Found(found) => {
                let domain = domain_of(&self.address).unwrap_or_default();
                let message = match &found {
                    Some(_) => tr!(
                        "Calendar found a CalDAV server for {domain}. Check that it belongs to your provider, then sign in.",
                        domain = domain
                    ),
                    None => tr!(
                        "Calendar found no CalDAV server for {domain}. Enter the server address your provider gives.",
                        domain = domain
                    ),
                };
                self.server = found.unwrap_or_default();
                if self.user.is_empty() {
                    self.user = self.address.clone();
                }
                self.step = Step::Server;
                self.present(ui);
                self.show_note(ui, &message);
                None
            }
            Answer::Signed(Err(error)) | Answer::Fetched(Err(error)) => {
                let message = problem(&error, self.step == Step::Subscription);
                self.fail(ui, &message);
                None
            }
            Answer::Signed(Ok(found)) => {
                let (server, user) = (self.server.clone(), self.user.clone());
                let password = self.password.clone().unwrap_or_default();
                self.close(ui);
                Some(match self.account.clone() {
                    Some(account) => Done::Updated {
                        account,
                        server,
                        user,
                        password,
                        found,
                    },
                    None => Done::Added {
                        server,
                        user,
                        password,
                        found,
                    },
                })
            }
            Answer::Fetched(Ok(data)) => {
                self.close(ui);
                Some(Done::Subscribed {
                    url: self.url.clone(),
                    data,
                })
            }
        }
    }

    fn close(&mut self, ui: &mut Ui) {
        self.closed = true;
        self.dismissed.set(true);
        ui.perform(&ActionRequest {
            action: Action::Click,
            target_tree: TreeId::ROOT,
            target_node: self.cancel.into(),
            data: None,
        });
    }
}
