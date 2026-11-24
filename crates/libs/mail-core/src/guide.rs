use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bande_l10n::tr;
use bande_toolkit::Rect;
use bande_toolkit::ui::{Event, FreeLayout, Key, Modifiers, Ui, Widget, WidgetId};
use zeroize::Zeroizing;

use crate::discover::{Settings, Source};
use crate::net::Network;
use crate::setup::{Found, Outcome, Problem, Stage, check, find};
use crate::sheet::{AccountSheet, Action, Field, Mode, Progress, Shared, SheetState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum News {
    Found(Found),
    Stage(Stage),
    Checked(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    run: u64,
    news: News,
}

pub type Reporter = Arc<dyn Fn(Report) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Sheet,
    Inline(WidgetId),
}

pub struct Options {
    pub icon: &'static [u8],
    pub offer_notes: bool,
    pub server_settings: bool,
    pub ready_note: String,
    pub taken: Vec<String>,
    pub network: Network,
    pub reporter: Reporter,
}

#[derive(Debug)]
pub enum Done {
    Cancelled,
    Ready {
        name: String,
        email: String,
        settings: Settings,
        password: Zeroizing<String>,
    },
    Finished {
        email: String,
        description: String,
        notes: bool,
    },
    Renewed {
        email: String,
        password: Zeroizing<String>,
    },
    ServerSettings,
}

pub struct Guide {
    options: Options,
    host: Host,
    state: Shared,
    mode: Mode,
    widget: WidgetId,
    run: u64,
    cancelled: Arc<AtomicBool>,
    manual: bool,
    stage: Stage,
}

fn next_run() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn lookup_line(domain: &str, source: Option<Source>) -> (String, Option<Progress>) {
    let domain = domain.to_owned();
    match source {
        None => (
            tr!("Looking up the servers of {domain}", domain = domain),
            Some(Progress::Running),
        ),
        Some(Source::Guess) => (
            tr!("Guessed the servers of {domain}", domain = domain),
            Some(Progress::Done),
        ),
        Some(Source::Services | Source::Domain) => (
            tr!("Found the servers {domain} publishes", domain = domain),
            Some(Progress::Done),
        ),
    }
}

fn stage_line(stage: Stage, settings: &Settings) -> String {
    match stage {
        Stage::Incoming => tr!(
            "Signing in to {server}",
            server = settings.imap.host.clone()
        ),
        Stage::Outgoing => tr!(
            "Checking sending via {server}",
            server = settings.smtp.host.clone()
        ),
    }
}

fn server_line(settings: &Settings) -> [(String, Option<Progress>); 2] {
    let describe = |server: &crate::net::Server| {
        tr!(
            "{host}, port {port}, {security}",
            host = server.host.clone(),
            port = server.port,
            security = server.security.title()
        )
    };
    [
        (
            tr!("Incoming: {server}", server = describe(&settings.imap)),
            None,
        ),
        (
            tr!("Outgoing: {server}", server = describe(&settings.smtp)),
            None,
        ),
    ]
}

impl Guide {
    pub fn open(ui: &mut Ui, host: Host, mode: Mode, options: Options) -> Self {
        let mut guide = Self {
            options,
            host,
            state: Rc::new(RefCell::new(SheetState::default())),
            mode: mode.clone(),
            widget: WidgetId::next(),
            run: 0,
            cancelled: Arc::new(AtomicBool::new(false)),
            manual: false,
            stage: Stage::Incoming,
        };
        guide.show(ui, mode);
        guide
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn host(&self) -> Host {
        self.host
    }

    pub fn widget(&self) -> WidgetId {
        self.widget
    }

    fn start_run(&mut self) -> Arc<AtomicBool> {
        self.stop_run();
        self.run = next_run();
        self.cancelled = Arc::new(AtomicBool::new(false));
        Arc::clone(&self.cancelled)
    }

    fn stop_run(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.run = 0;
    }

    fn show(&mut self, ui: &mut Ui, mode: Mode) {
        let inline = matches!(self.host, Host::Inline(_));
        let sheet = AccountSheet::new(Rc::clone(&self.state), mode.clone(), &self.options, inline);
        let focus = self
            .state
            .borrow()
            .problem
            .as_ref()
            .and_then(|(field, _)| sheet.field(*field))
            .or_else(|| sheet.first_field().filter(|_| inline));
        let id = sheet.id();
        match self.host {
            Host::Sheet if ui.widget_ref::<AccountSheet>(self.widget).is_some() => {
                ui.replace_sheet(sheet);
            }
            Host::Sheet => ui.present_sheet(sheet),
            Host::Inline(layout) => {
                if let Some(layout) = ui.widget_as::<FreeLayout>(layout) {
                    match layout.frame_of(self.widget) {
                        Some(_) => {
                            layout.replace(self.widget, sheet);
                        }
                        None => {
                            layout.insert(sheet, Rect::default());
                        }
                    }
                }
            }
        }
        self.widget = id;
        self.mode = mode;
        match focus {
            Some(field) => ui.set_focus(field),
            None if inline => ui.clear_focus(),
            None => {}
        }
    }

    pub fn place(&self, ui: &mut Ui, frame: Rect) {
        if let Host::Inline(layout) = self.host
            && let Some(layout) = ui.widget_as::<FreeLayout>(layout)
        {
            layout.set_frame(self.widget, frame);
        }
    }

    pub fn close(mut self, ui: &mut Ui) {
        self.stop_run();
        self.state.borrow_mut().closed = true;
        if let Host::Inline(layout) = self.host
            && let Some(layout) = ui.widget_as::<FreeLayout>(layout)
        {
            layout.remove(self.widget);
            ui.clear_focus();
        }
    }

    fn submit_password(&self, ui: &mut Ui) {
        let Some(field) = ui
            .widget_as::<AccountSheet>(self.widget)
            .and_then(|sheet| sheet.field(Field::Password))
        else {
            return;
        };
        let previous = ui.focus();
        ui.set_focus(field);
        ui.handle(&Event::Key {
            key: Key::Enter,
            modifiers: Modifiers::default(),
        });
        match previous {
            Some(previous) => ui.set_focus(previous),
            None => ui.clear_focus(),
        }
        self.state.borrow_mut().action = None;
    }

    fn clear(&self) {
        let mut state = self.state.borrow_mut();
        state.lines.clear();
        state.problem = None;
        state.status = None;
    }

    fn problem(&self, field: Field, text: String) {
        let mut state = self.state.borrow_mut();
        state.problem = Some((field, text));
    }

    fn address_problem(&self) -> Option<(Field, String)> {
        let state = self.state.borrow();
        let email = state.value(Field::Email);
        if state.value(Field::Name).is_empty() {
            Some((Field::Name, tr!("Enter your name.")))
        } else if bande_discovery::domain_of(&email).is_none() {
            Some((Field::Email, tr!("Enter your email address.")))
        } else if self
            .options
            .taken
            .iter()
            .any(|taken| taken.eq_ignore_ascii_case(&email))
        {
            Some((Field::Email, tr!("This account is already set up.")))
        } else if state.password.is_empty() {
            Some((Field::Password, tr!("Enter your password.")))
        } else {
            None
        }
    }

    pub fn act(&mut self, ui: &mut Ui) -> Option<Done> {
        let action = self.state.borrow_mut().action.take()?;
        if action != Action::Cancel {
            self.submit_password(ui);
        }
        match (action, self.mode.clone()) {
            (Action::Cancel, Mode::Checking) => {
                self.stop_run();
                let back = match self.manual {
                    true => Mode::Servers,
                    false => Mode::Address,
                };
                self.state.borrow_mut().lines.clear();
                self.show(ui, back);
                None
            }
            (Action::Cancel, Mode::Confirm(_) | Mode::Servers)
                if matches!(self.host, Host::Inline(_)) =>
            {
                self.clear();
                self.show(ui, Mode::Address);
                None
            }
            (Action::Cancel, _) => Some(Done::Cancelled),
            (Action::Primary, Mode::Address) => {
                self.manual = false;
                self.look_up(ui);
                None
            }
            (Action::Primary, Mode::Servers) => {
                self.manual = true;
                self.sign_in(ui);
                None
            }
            (Action::Primary, Mode::Confirm(_)) => {
                self.sign_in(ui);
                None
            }
            (Action::Primary, Mode::Added(email)) => {
                let state = self.state.borrow();
                Some(Done::Finished {
                    email,
                    description: state.value(Field::Description),
                    notes: self.options.offer_notes && state.notes(),
                })
            }
            (Action::Primary, Mode::Password(email)) => {
                let password = self.state.borrow().password.clone();
                if password.is_empty() {
                    self.problem(Field::Password, tr!("Enter your password."));
                    self.show(ui, Mode::Password(email));
                    return None;
                }
                Some(Done::Renewed { email, password })
            }
            (Action::Other, Mode::Address | Mode::Confirm(_)) => {
                self.clear();
                self.show(ui, Mode::Servers);
                None
            }
            (Action::Other, Mode::Servers) => {
                self.clear();
                self.show(ui, Mode::Address);
                None
            }
            (Action::Other, Mode::Password(_)) => Some(Done::ServerSettings),
            (Action::Primary | Action::Other, Mode::Checking | Mode::Added(_)) => None,
        }
    }

    fn look_up(&mut self, ui: &mut Ui) {
        let problem = self.address_problem();
        {
            let mut state = self.state.borrow_mut();
            state.problem = problem.clone();
            state.status = None;
        }
        if problem.is_some() {
            self.show(ui, Mode::Address);
            return;
        }
        let email = self.state.borrow().value(Field::Email);
        let domain = bande_discovery::domain_of(&email).unwrap_or_default();
        self.state.borrow_mut().lines = vec![lookup_line(&domain, None)];
        let cancelled = self.start_run();
        let (run, network, reporter) = (
            self.run,
            self.options.network.clone(),
            Arc::clone(&self.options.reporter),
        );
        std::thread::spawn(move || {
            let found = find(&email, &network, &cancelled);
            if !cancelled.load(Ordering::Relaxed) {
                reporter(Report {
                    run,
                    news: News::Found(found),
                });
            }
        });
        self.show(ui, Mode::Checking);
    }

    fn sign_in(&mut self, ui: &mut Ui) {
        let servers = self.state.borrow().servers();
        let checked = servers.and_then(|servers| match self.address_problem() {
            Some(problem) => Err(problem),
            None => Ok(servers),
        });
        let servers = match checked {
            Ok(servers) => servers,
            Err(problem) => {
                let mut state = self.state.borrow_mut();
                state.lines.clear();
                state.problem = Some(problem);
                drop(state);
                self.show(ui, Mode::Servers);
                return;
            }
        };
        {
            let mut state = self.state.borrow_mut();
            state.problem = None;
            state.status = None;
            state
                .lines
                .retain(|(_, progress)| *progress == Some(Progress::Done));
            state.lines.truncate(1);
            if self.manual {
                state.lines.clear();
            }
            for stage in [Stage::Incoming, Stage::Outgoing] {
                state
                    .lines
                    .push((stage_line(stage, &servers), Some(Progress::Waiting)));
            }
        }
        self.check(servers);
        if self.mode != Mode::Checking {
            self.show(ui, Mode::Checking);
        }
    }

    fn check(&mut self, servers: Settings) {
        let cancelled = self.start_run();
        self.stage = Stage::Incoming;
        let password = Zeroizing::new(self.state.borrow().password.to_string());
        let (run, network, reporter) = (
            self.run,
            self.options.network.clone(),
            Arc::clone(&self.options.reporter),
        );
        std::thread::spawn(move || {
            let outcome = check(&servers, &password, &network, &cancelled, |stage| {
                reporter(Report {
                    run,
                    news: News::Stage(stage),
                });
            });
            reporter(Report {
                run,
                news: News::Checked(outcome),
            });
        });
    }

    pub fn report(&mut self, ui: &mut Ui, report: Report) -> Option<Done> {
        if report.run != self.run || self.mode != Mode::Checking {
            return None;
        }
        match report.news {
            News::Found(found) => {
                self.found(ui, found);
                None
            }
            News::Stage(stage) => {
                self.stage = stage;
                let running = (self.state.borrow().lines.len() + stage as usize).saturating_sub(2);
                for (index, (_, progress)) in self.state.borrow_mut().lines.iter_mut().enumerate() {
                    if progress.is_some() {
                        *progress = Some(match index.cmp(&running) {
                            std::cmp::Ordering::Less => Progress::Done,
                            std::cmp::Ordering::Equal => Progress::Running,
                            std::cmp::Ordering::Greater => Progress::Waiting,
                        });
                    }
                }
                None
            }
            News::Checked(Outcome::Ready(settings)) => {
                for (_, progress) in self.state.borrow_mut().lines.iter_mut() {
                    *progress = progress.map(|_| Progress::Done);
                }
                let state = self.state.borrow();
                Some(Done::Ready {
                    name: state.value(Field::Name),
                    email: state.value(Field::Email),
                    settings,
                    password: state.password.clone(),
                })
            }
            News::Checked(Outcome::Failed(settings, problem)) => {
                self.failed(ui, &settings, problem);
                None
            }
            News::Checked(Outcome::Stopped) => None,
        }
    }

    fn found(&mut self, ui: &mut Ui, found: Found) {
        let Some((settings, source)) = found.settings.clone() else {
            let mut state = self.state.borrow_mut();
            state.lines.clear();
            state.values.retain(|(field, _)| {
                matches!(
                    field,
                    Field::Name | Field::Email | Field::Description | Field::Notes
                )
            });
            state.status = Some((found.message(), false));
            drop(state);
            self.show(ui, Mode::Servers);
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            state.fill(&settings);
            state.lines = vec![lookup_line(&found.domain, Some(source))];
        }
        if found.trusted() {
            self.sign_in(ui);
            return;
        }
        let message = match source {
            Source::Guess => found.message(),
            Source::Services | Source::Domain => tr!(
                "Mail for {domain} is handled by servers outside that domain. Sign in there only if they belong to your mail provider.",
                domain = found.domain.clone()
            ),
        };
        self.state.borrow_mut().lines = server_line(&settings).to_vec();
        self.show(ui, Mode::Confirm(message));
    }

    fn failed(&mut self, ui: &mut Ui, settings: &Settings, problem: Problem) {
        let password_problem = matches!(
            problem,
            Problem::WrongCredentials(_) | Problem::NoPasswordSignIn(_)
        );
        {
            let mut state = self.state.borrow_mut();
            state.lines.clear();
            state.fill(settings);
            let field = match (password_problem, self.stage) {
                (true, _) => Field::Password,
                (false, Stage::Incoming) => Field::Incoming,
                (false, Stage::Outgoing) => Field::Outgoing,
            };
            if password_problem {
                state.password = Zeroizing::default();
                state.shake = true;
            }
            state.problem = Some((field, problem.message()));
        }
        let back = match password_problem && !self.manual {
            true => Mode::Address,
            false => Mode::Servers,
        };
        self.show(ui, back);
    }

    pub fn added(&mut self, ui: &mut Ui, email: String) {
        let mut state = self.state.borrow_mut();
        state.lines.clear();
        state.problem = None;
        state.status = Some(self.options.ready_note.clone())
            .filter(|note| !note.is_empty())
            .map(|note| (note, false));
        state.password = Zeroizing::default();
        drop(state);
        self.show(ui, Mode::Added(email));
    }

    pub fn tell(&mut self, ui: &mut Ui, error: String) {
        self.state.borrow_mut().lines.clear();
        self.state.borrow_mut().status = Some((error, true));
        let back = match self.mode {
            Mode::Checking if self.manual => Mode::Servers,
            Mode::Checking => Mode::Address,
            ref mode => mode.clone(),
        };
        self.show(ui, back);
    }
}
