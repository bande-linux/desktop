use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use bande_dav::{Credentials, Error, FoundAddressBooks};
use bande_discovery::domain_of;
use bande_l10n::tr;
use bande_toolkit::ui::{
    Alert, CheckState, Checkbox, Closable, Form, FormLine, SecureField, Spinner, TextField, Ui,
    Widget, WidgetId,
};
use zeroize::Zeroizing;

use crate::worker::Job;

const FIELD_FONT: f32 = 13.0;

static NEXT_SETUP: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Address,
    Server,
    Password,
}

/// What the account sheet ends with, for the app to keep.
#[derive(Debug)]
pub enum Done {
    Added {
        address: String,
        server: String,
        credentials: Credentials,
        found: FoundAddressBooks,
        also_calendar: bool,
    },
    Updated {
        account: String,
        server: String,
        credentials: Credentials,
        found: FoundAddressBooks,
    },
    Password {
        account: String,
        credentials: Credentials,
    },
}

#[derive(Default)]
struct Fields {
    address: Option<WidgetId>,
    server: Option<WidgetId>,
    user: Option<WidgetId>,
    password: Option<WidgetId>,
    also: Option<WidgetId>,
}

/// The CardDAV account sheet: the address and password, the server found from the address's
/// own domain (RFC 6764) to confirm or change, or the server entered by hand; and, when the
/// server refuses a saved password, the password again.
pub struct Setup {
    id: u64,
    step: Step,
    account: Option<String>,
    name: String,
    address: String,
    server: String,
    user: String,
    password: Zeroizing<String>,
    also_calendar: bool,
    calendar: bool,
    fields: Fields,
    busy: Option<Rc<Cell<bool>>>,
    choices: Rc<RefCell<Vec<String>>>,
}

pub fn problem(error: &Error) -> String {
    match error {
        Error::Network(_) => tr!(
            "Contacts could not reach the server. Check the address, and that this computer is online."
        ),
        Error::Certificate(server) => tr!(
            "The certificate of {server} could not be verified. Check that the address is exactly the one your provider gives.",
            server = server.clone()
        ),
        Error::Unauthorized => tr!(
            "The server did not accept the user name or password. Check both; some providers need a separate app password."
        ),
        Error::Unsupported(schemes) => tr!(
            "The server asks for a sign-in method Contacts does not support ({schemes}). Ask your provider for sign-in with a password.",
            schemes = schemes.clone()
        ),
        Error::Insecure(server) => tr!(
            "{server} offers no encrypted connection, so Contacts does not send the password. Use an https:// address.",
            server = server.clone()
        ),
        Error::Status { status, .. } if *status >= 500 => tr!(
            "The server answered with an error ({status}). Try again later.",
            status = *status
        ),
        _ => tr!(
            "There is no CardDAV address book at this address. Check the server address and path your provider gives."
        ),
    }
}

fn boxed(widget: impl Widget + 'static) -> Box<dyn Widget> {
    Box::new(widget)
}

impl Setup {
    fn with(step: Step, calendar: bool) -> Self {
        Self {
            id: NEXT_SETUP.fetch_add(1, Ordering::Relaxed),
            step,
            account: None,
            name: String::new(),
            address: String::new(),
            server: String::new(),
            user: String::new(),
            password: Zeroizing::default(),
            also_calendar: calendar,
            calendar,
            fields: Fields::default(),
            busy: None,
            choices: Rc::default(),
        }
    }

    /// A sheet that adds an account; `calendar` offers to use it for Calendar too.
    pub fn add(calendar: bool) -> Self {
        Self::with(Step::Address, calendar)
    }

    /// A sheet for an account whose server refused the saved password.
    pub fn password(account: &str, name: &str, user: &str, server: &str) -> Self {
        Self {
            account: Some(account.to_owned()),
            name: name.to_owned(),
            user: user.to_owned(),
            server: server.to_owned(),
            ..Self::with(Step::Password, false)
        }
    }

    /// A sheet that checks and changes an account's server settings.
    pub fn edit(account: &str, name: &str, user: &str, server: &str) -> Self {
        Self {
            step: Step::Server,
            ..Self::password(account, name, user, server)
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    fn text_field(&mut self, placeholder: &str, value: &str) -> (TextField, WidgetId) {
        let field = TextField::new(placeholder, value, |_| {}).without_text_checking();
        let id = field.id();
        (field, id)
    }

    fn password_line(&mut self, lines: &mut Vec<FormLine>) {
        let field = SecureField::new(&tr!("Required"), FIELD_FONT, |_| {});
        self.fields.password = Some(field.id());
        lines.push(FormLine::Row(tr!("Password"), boxed(field)));
    }

    fn also_line(&mut self, lines: &mut Vec<FormLine>) {
        if !self.calendar || self.account.is_some() {
            return;
        }
        let state = match self.also_calendar {
            true => CheckState::On,
            false => CheckState::Off,
        };
        let also = Checkbox::new(&tr!("Also use this account for Calendar"), state, |_| {});
        self.fields.also = Some(also.id());
        lines.push(FormLine::Wide(boxed(also)));
    }

    /// Shows the sheet's current step, with `note` (what was found, or what went wrong).
    pub fn present(&mut self, ui: &mut Ui, note: Option<&str>) {
        self.fields = Fields::default();
        let mut lines = Vec::new();
        let (title, message) = match self.step {
            Step::Address => {
                let (field, id) = self.text_field(&tr!("jane@example.org"), &self.address.clone());
                self.fields.address = Some(id);
                lines.push(FormLine::Row(tr!("Address"), boxed(field)));
                self.password_line(&mut lines);
                self.also_line(&mut lines);
                (
                    tr!("Add a CardDAV Account"),
                    tr!(
                        "Contacts looks up the CardDAV server of your address’s domain and shows it to you before it signs in."
                    ),
                )
            }
            Step::Server => {
                let (server, id) =
                    self.text_field(&tr!("https://dav.example.org/"), &self.server.clone());
                self.fields.server = Some(id);
                lines.push(FormLine::Row(tr!("Server"), boxed(server)));
                let (user, id) = self.text_field(&tr!("jane@example.org"), &self.user.clone());
                self.fields.user = Some(id);
                lines.push(FormLine::Row(tr!("User name"), boxed(user)));
                self.password_line(&mut lines);
                self.also_line(&mut lines);
                let title = match &self.account {
                    Some(_) => tr!("Sign in to “{name}”", name = self.name.clone()),
                    None => tr!("CardDAV Server Settings"),
                };
                (
                    title,
                    tr!(
                        "Enter the server address your provider gives, with its port and path if it has them. Contacts sends the password only when the server asks for it."
                    ),
                )
            }
            Step::Password => {
                self.password_line(&mut lines);
                (
                    tr!("Sign in to “{name}”", name = self.name.clone()),
                    tr!(
                        "The server did not accept the password for {user}. Enter it again, or check the server settings.",
                        user = self.user.clone()
                    ),
                )
            }
        };
        let message = note.map_or(message, str::to_owned);
        let choices = Rc::clone(&self.choices);
        let mut alert = Alert::new(&title, &message, move |action| {
            choices.borrow_mut().push(action.to_owned());
        })
        .accessory(Form::new(&title, lines));
        alert = match (self.step, &self.account) {
            (Step::Address, _) => alert.other(&tr!("Enter Server Settings Manually"), "manual"),
            (Step::Server, None) => alert.other(&tr!("Back"), "back"),
            (Step::Password, _) => alert.other(&tr!("Server Settings…"), "settings"),
            (Step::Server, Some(_)) => alert,
        };
        let primary = match self.step {
            Step::Address => tr!("Continue"),
            Step::Server | Step::Password => tr!("Sign In"),
        };
        ui.present_sheet(
            alert
                .cancel(&tr!("Cancel"), "cancel")
                .primary(&primary, "primary"),
        );
        let first = match self.step {
            Step::Address => self.fields.address,
            Step::Server => self.fields.server,
            Step::Password => self.fields.password,
        };
        if let Some(first) = first {
            ui.set_focus(first);
        }
    }

    fn read(&mut self, ui: &Ui) {
        let text = |id: Option<WidgetId>| {
            id.and_then(|id| ui.widget_ref::<TextField>(id))
                .map(|field| field.text().trim().to_owned())
        };
        if let Some(address) = text(self.fields.address) {
            self.address = address;
        }
        if let Some(server) = text(self.fields.server) {
            self.server = server;
        }
        if let Some(user) = text(self.fields.user) {
            self.user = user;
        }
        if let Some(secret) = self
            .fields
            .password
            .and_then(|id| ui.widget_ref::<SecureField>(id))
        {
            self.password = Zeroizing::new(secret.secret().to_owned());
        }
        if let Some(also) = self
            .fields
            .also
            .and_then(|id| ui.widget_ref::<Checkbox>(id))
        {
            self.also_calendar = also.state() == CheckState::On;
        }
    }

    fn working(&mut self, ui: &mut Ui, status: &str) {
        let closed = Rc::new(Cell::new(false));
        let choices = Rc::clone(&self.choices);
        let alert = Alert::new(status, "", move |action| {
            choices.borrow_mut().push(action.to_owned());
        })
        .accessory(Spinner::new(status))
        .cancel(&tr!("Cancel"), "stop");
        ui.present_sheet(Closable::new(alert, Rc::clone(&closed)));
        self.busy = Some(closed);
    }

    fn credentials(&self) -> Credentials {
        Credentials {
            user: self.user.clone(),
            password: self.password.to_string(),
        }
    }

    /// Handles the buttons pressed in the sheet: the job to run on the network, if any, and
    /// whether the sheet is finished.
    pub fn update(&mut self, ui: &mut Ui) -> (Option<Job>, Option<Done>, bool) {
        let choices: Vec<String> = self.choices.borrow_mut().drain(..).collect();
        let mut job = None;
        for choice in choices {
            if choice != "stop" {
                self.read(ui);
            }
            match (choice.as_str(), self.step) {
                ("cancel", _) => return (None, None, true),
                ("stop", _) => {
                    self.busy = None;
                    self.present(ui, None);
                }
                ("manual", _) => {
                    if self.user.is_empty() {
                        self.user = self.address.clone();
                    }
                    self.step = Step::Server;
                    self.present(ui, None);
                }
                ("back", _) => {
                    self.step = Step::Address;
                    self.present(ui, None);
                }
                ("settings", _) => {
                    self.step = Step::Server;
                    self.present(ui, None);
                }
                ("primary", Step::Address) => {
                    if domain_of(&self.address).is_none() {
                        self.present(
                            ui,
                            Some(&tr!("Enter your address, such as jane@example.org.")),
                        );
                    } else {
                        self.working(ui, &tr!("Looking up the server…"));
                        job = Some(Job::Find {
                            setup: self.id,
                            address: self.address.clone(),
                        });
                    }
                }
                ("primary", Step::Password) if self.password.is_empty() => {
                    self.present(ui, Some(&tr!("Enter your password.")));
                }
                ("primary", Step::Password) => {
                    let Some(account) = self.account.clone() else {
                        return (None, None, true);
                    };
                    return (
                        None,
                        Some(Done::Password {
                            account,
                            credentials: self.credentials(),
                        }),
                        true,
                    );
                }
                ("primary", Step::Server) => {
                    let missing = match () {
                        _ if self.server.is_empty() => {
                            Some(tr!("Enter the address of the server."))
                        }
                        _ if self.user.is_empty() => Some(tr!("Enter your user name.")),
                        _ if self.password.is_empty() => Some(tr!("Enter your password.")),
                        _ => None,
                    };
                    match missing {
                        Some(missing) => self.present(ui, Some(&missing)),
                        None => {
                            self.working(ui, &tr!("Signing in…"));
                            job = Some(Job::SignIn {
                                setup: self.id,
                                server: self.server.clone(),
                                credentials: self.credentials(),
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        (job, None, false)
    }

    fn settle(&mut self) -> bool {
        match self.busy.take() {
            Some(closed) => {
                closed.set(true);
                true
            }
            None => false,
        }
    }

    /// Takes the server the address's domain points to.
    pub fn found(&mut self, ui: &mut Ui, server: Option<String>) {
        if !self.settle() {
            return;
        }
        let domain = domain_of(&self.address).unwrap_or_default();
        let note = match &server {
            Some(_) => tr!(
                "Contacts found a CardDAV server for {domain}. Check that it belongs to your provider, then sign in.",
                domain = domain
            ),
            None => tr!(
                "Contacts found no CardDAV server for {domain}. Enter the server address your provider gives.",
                domain = domain
            ),
        };
        self.server = server.unwrap_or_default();
        if self.user.is_empty() {
            self.user = self.address.clone();
        }
        self.step = Step::Server;
        self.present(ui, Some(&note));
    }

    /// Takes the answer of a sign-in: the account to keep, or the sheet again with what went
    /// wrong and every field as it was.
    pub fn signed(
        &mut self,
        ui: &mut Ui,
        result: Result<FoundAddressBooks, Error>,
    ) -> Option<Done> {
        if !self.settle() {
            return None;
        }
        match (result, self.account.clone()) {
            (Err(error), _) => {
                self.present(ui, Some(&problem(&error)));
                None
            }
            (Ok(found), Some(account)) => Some(Done::Updated {
                account,
                server: self.server.clone(),
                credentials: self.credentials(),
                found,
            }),
            (Ok(found), None) => Some(Done::Added {
                address: match self.address.is_empty() {
                    true => self.user.clone(),
                    false => self.address.clone(),
                },
                server: self.server.clone(),
                credentials: self.credentials(),
                found,
                also_calendar: self.also_calendar,
            }),
        }
    }
}
