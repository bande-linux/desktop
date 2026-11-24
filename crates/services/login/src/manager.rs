use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use bande_pam::{Backoff, Code};

use zeroize::Zeroizing;

use crate::protocol::{
    AuthMessageType, Class, ErrorType, FromWorker, Request, Response, ToWorker, carried_environment,
};

/// The login window's own account.
pub const GREETER_USER: &str = "_bande-greeter";
/// How long the login window stays on screen before its end counts as a new failure rather than
/// one of a row.
pub const SETTLED: Duration = Duration::from_secs(60);
const FAILURES_ALLOWED: u32 = 3;
const USERNAME_LIMIT: usize = 256;
/// How long the login manager waits before it starts a login window again that was never shown.
pub const RESPAWN_PAUSE: Duration = Duration::from_secs(2);
/// How long workers that are ending may hold up the console once the login window has failed.
pub const FAILED_EXIT_LIMIT: Duration = Duration::from_secs(15);

pub type WorkerId = u64;

/// What the daemon does next, as the [`Manager`] decides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Starts a session worker and sends it `Begin` for `class` and `username`.
    Spawn {
        worker: WorkerId,
        class: Class,
        username: String,
    },
    Tell(WorkerId, ToWorker),
    /// Lets go of a worker, which deletes the credentials it established and ends the session it
    /// opened.
    Stop(WorkerId),
    /// Ends a worker that has established nothing yet, in the middle of its conversation.
    Kill(WorkerId),
    /// Answers the login window's last request.
    Reply(Response),
    /// Brings the login window's virtual terminal to the front.
    ShowGreeter,
    Exit {
        success: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Authenticating,
    Authenticated,
    Starting,
}

#[derive(Debug)]
struct Login {
    worker: WorkerId,
    account: Option<String>,
    phase: Phase,
    answered: bool,
    accepted: bool,
    spoke: bool,
    confirming: bool,
}

#[derive(Debug)]
struct Greeter {
    worker: WorkerId,
    shown: Option<Instant>,
    stopping: bool,
}

/// The login manager's decisions: which sessions run, which worker the login window talks to,
/// and what happens when either side goes away.
#[derive(Debug, Default)]
pub struct Manager {
    next: WorkerId,
    greeter: Option<Greeter>,
    login: Option<Login>,
    sessions: Vec<WorkerId>,
    abandoned: Vec<Login>,
    waiting: bool,
    asked: bool,
    queued: VecDeque<Response>,
    failures: u32,
    attempts: HashMap<String, Backoff>,
    ending: bool,
    failed: bool,
    respawn_at: Option<Instant>,
    exit_by: Option<Instant>,
}

fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= USERNAME_LIMIT
        && !username.starts_with(['-', '+'])
        && !username.chars().any(char::is_control)
}

impl Manager {
    fn spawn(&mut self, class: Class, username: &str) -> (WorkerId, Action) {
        self.next += 1;
        let worker = self.next;
        (
            worker,
            Action::Spawn {
                worker,
                class,
                username: username.to_owned(),
            },
        )
    }

    fn spawn_greeter(&mut self) -> Action {
        let (worker, action) = self.spawn(Class::Greeter, GREETER_USER);
        self.greeter = Some(Greeter {
            worker,
            shown: None,
            stopping: false,
        });
        action
    }

    pub fn start(&mut self) -> Vec<Action> {
        vec![self.spawn_greeter()]
    }

    fn respond(&mut self, response: Response) -> Vec<Action> {
        match self.waiting {
            true => self.deliver(response),
            false => {
                self.queued.push_back(response);
                Vec::new()
            }
        }
    }

    fn deliver(&mut self, response: Response) -> Vec<Action> {
        self.waiting = false;
        self.asked = matches!(
            response,
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret | AuthMessageType::Visible,
                ..
            }
        );
        vec![Action::Reply(response)]
    }

    fn await_worker(&mut self) -> Vec<Action> {
        self.waiting = true;
        match self.queued.pop_front() {
            Some(response) => self.deliver(response),
            None => Vec::new(),
        }
    }

    fn attempt_failed(&mut self, login: &Login, now: Instant) {
        if login.answered
            && !login.accepted
            && let Some(account) = &login.account
        {
            self.attempts.entry(account.clone()).or_default().fail(now);
        }
    }

    fn abandon(&mut self, login: Login, now: Instant) -> Action {
        let worker = login.worker;
        let action = match login.phase {
            Phase::Authenticating if !login.accepted => {
                self.attempt_failed(&login, now);
                Action::Kill(worker)
            }
            _ => Action::Stop(worker),
        };
        self.abandoned.push(login);
        action
    }

    fn drop_conversation(&mut self, now: Instant) -> Vec<Action> {
        self.queued.clear();
        self.asked = false;
        match self.login.take() {
            Some(login) if login.phase == Phase::Starting => {
                self.login = Some(login);
                Vec::new()
            }
            Some(login) => vec![self.abandon(login, now)],
            None => Vec::new(),
        }
    }

    fn answer(&mut self, response: Option<Zeroizing<String>>, now: Instant) -> Vec<Action> {
        let Some(login) = self
            .login
            .as_mut()
            .filter(|login| login.phase == Phase::Authenticating)
        else {
            if !self.queued.is_empty() {
                return self.await_worker();
            }
            return vec![Action::Reply(Response::error(
                ErrorType::Error,
                "nothing was asked",
            ))];
        };
        let mut actions = Vec::new();
        if std::mem::take(&mut login.confirming) {
            actions.push(Action::Tell(login.worker, ToWorker::Proceed));
        } else if std::mem::take(&mut self.asked) {
            let throttled = !login.accepted
                && login
                    .account
                    .as_ref()
                    .and_then(|account| self.attempts.get(account))
                    .is_some_and(|backoff| backoff.waiting(now).is_some());
            if throttled {
                let mut actions = self.drop_conversation(now);
                actions.push(Action::Reply(Response::error(
                    ErrorType::AuthError,
                    format!("too many attempts: {}", Code::MAXTRIES),
                )));
                return actions;
            }
            login.answered = true;
            actions.push(Action::Tell(login.worker, ToWorker::Answer { response }));
        }
        actions.extend(self.await_worker());
        actions
    }

    /// Handles a request from the login window. Each request is answered once, in order: one that
    /// arrives while an earlier one still waits for its answer takes its place.
    pub fn request(&mut self, request: Request, now: Instant) -> Vec<Action> {
        let superseded = std::mem::take(&mut self.waiting).then(|| {
            Action::Reply(Response::error(
                ErrorType::Error,
                "another request took its place",
            ))
        });
        let mut actions: Vec<Action> = superseded.into_iter().collect();
        actions.extend(self.handle(request, now));
        actions
    }

    fn handle(&mut self, request: Request, now: Instant) -> Vec<Action> {
        if self.ending {
            return vec![Action::Reply(Response::error(
                ErrorType::Error,
                "the login manager is stopping",
            ))];
        }
        match request {
            Request::CreateSession { username } => {
                if self
                    .login
                    .as_ref()
                    .is_some_and(|login| login.phase == Phase::Starting)
                {
                    return vec![Action::Reply(Response::error(
                        ErrorType::Error,
                        "a session is starting",
                    ))];
                }
                if !valid_username(&username) {
                    return vec![Action::Reply(Response::error(
                        ErrorType::AuthError,
                        "pam_start: USER_UNKNOWN",
                    ))];
                }
                let mut actions = self.drop_conversation(now);
                let (worker, spawn) = self.spawn(Class::User, &username);
                self.login = Some(Login {
                    worker,
                    account: None,
                    phase: Phase::Authenticating,
                    answered: false,
                    accepted: false,
                    spoke: false,
                    confirming: false,
                });
                actions.push(spawn);
                actions.extend(self.await_worker());
                actions
            }
            Request::PostAuthMessageResponse { response } => self.answer(response, now),
            Request::StartSession { env } => {
                let Some(login) = self
                    .login
                    .as_mut()
                    .filter(|login| login.phase == Phase::Authenticated)
                else {
                    return vec![Action::Reply(Response::error(
                        ErrorType::Error,
                        "no one has signed in",
                    ))];
                };
                login.phase = Phase::Starting;
                let worker = login.worker;
                let env = carried_environment(env);
                let mut actions = vec![Action::Tell(worker, ToWorker::Launch { env })];
                actions.extend(self.await_worker());
                actions
            }
            Request::CancelSession => {
                let mut actions: Vec<Action> = self
                    .login
                    .take()
                    .map(|login| self.abandon(login, now))
                    .into_iter()
                    .collect();
                self.queued.clear();
                self.asked = false;
                actions.push(Action::Reply(Response::Success));
                actions
            }
        }
    }

    /// Forgets the login window's connection, abandoning a conversation that has not yet
    /// started a session.
    pub fn disconnected(&mut self, now: Instant) -> Vec<Action> {
        self.waiting = false;
        self.drop_conversation(now)
    }

    /// Handles what a worker said.
    pub fn worker_said(&mut self, worker: WorkerId, said: FromWorker, now: Instant) -> Vec<Action> {
        if said == FromWorker::Shown
            && let Some(greeter) = self.greeter.as_mut()
            && greeter.worker == worker
        {
            greeter.shown = Some(now);
            return Vec::new();
        }
        let Some(login) = self.login.as_mut().filter(|login| login.worker == worker) else {
            return Vec::new();
        };
        match (login.phase, said) {
            (Phase::Authenticating, FromWorker::Account { name }) => {
                login.account = Some(name);
                Vec::new()
            }
            (Phase::Authenticating, FromWorker::Accepted) => {
                login.accepted = true;
                if let Some(account) = &login.account {
                    self.attempts.remove(account);
                }
                login.confirming = !login.spoke;
                match login.confirming {
                    true => self.respond(Response::AuthMessage {
                        auth_message_type: AuthMessageType::Confirm,
                        auth_message: String::new(),
                    }),
                    false => vec![Action::Tell(worker, ToWorker::Proceed)],
                }
            }
            (Phase::Authenticating, FromWorker::Ask { text, visible }) => {
                login.spoke = true;
                let auth_message_type = match visible {
                    true => AuthMessageType::Visible,
                    false => AuthMessageType::Secret,
                };
                self.respond(Response::AuthMessage {
                    auth_message_type,
                    auth_message: text,
                })
            }
            (Phase::Authenticating, FromWorker::Show { text, error }) => {
                login.spoke = true;
                let auth_message_type = match error {
                    true => AuthMessageType::Error,
                    false => AuthMessageType::Info,
                };
                self.respond(Response::AuthMessage {
                    auth_message_type,
                    auth_message: text,
                })
            }
            (Phase::Authenticating, FromWorker::Authenticated) => {
                login.phase = Phase::Authenticated;
                self.respond(Response::Success)
            }
            (
                _,
                FromWorker::Failed {
                    error_type,
                    description,
                },
            ) => {
                if let Some(login) = self.login.take() {
                    if error_type == ErrorType::AuthError {
                        self.attempt_failed(&login, now);
                    }
                    self.abandoned.push(login);
                }
                self.respond(Response::Error {
                    error_type,
                    description,
                })
            }
            (Phase::Starting, FromWorker::Shown) => {
                self.login = None;
                self.failures = 0;
                self.sessions.push(worker);
                let mut actions = self.respond(Response::Success);
                if let Some(greeter) = self.greeter.as_mut().filter(|greeter| !greeter.stopping) {
                    greeter.stopping = true;
                    actions.push(Action::Stop(greeter.worker));
                }
                actions
            }
            (Phase::Authenticated | Phase::Starting, FromWorker::Ask { .. }) => {
                vec![Action::Tell(worker, ToWorker::Answer { response: None })]
            }
            (phase, said) => {
                tracing::warn!("a session worker said {said:?} while {phase:?}");
                Vec::new()
            }
        }
    }

    fn needs_greeter(&self) -> bool {
        !self.ending && self.greeter.is_none() && self.login.is_none() && self.sessions.is_empty()
    }

    fn finished(&self) -> bool {
        self.greeter.is_none()
            && self.login.is_none()
            && self.sessions.is_empty()
            && self.abandoned.is_empty()
    }

    fn greeter_ended(&mut self, greeter: &Greeter, now: Instant) -> Vec<Action> {
        if greeter.stopping || self.ending {
            return Vec::new();
        }
        tracing::warn!("the login window ended");
        let settled = greeter
            .shown
            .is_some_and(|shown| now.saturating_duration_since(shown) >= SETTLED);
        self.failures = match settled {
            true => 1,
            false => self.failures + 1,
        };
        let actions = self.drop_conversation(now);
        if self.failures >= FAILURES_ALLOWED && self.login.is_none() && self.sessions.is_empty() {
            self.ending = true;
            self.failed = true;
            self.exit_by = Some(now + FAILED_EXIT_LIMIT);
        } else if greeter.shown.is_none() {
            self.respawn_at = Some(now + RESPAWN_PAUSE);
        }
        actions
    }

    fn gives_the_screen_back(&self, ended: &Login) -> bool {
        ended.phase == Phase::Starting
            && !self.ending
            && self.login.is_none()
            && self.sessions.is_empty()
    }

    /// Handles a worker that has ended.
    pub fn worker_ended(&mut self, worker: WorkerId, now: Instant) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(greeter) = self.greeter.take_if(|greeter| greeter.worker == worker) {
            actions.extend(self.greeter_ended(&greeter, now));
        } else if let Some(login) = self.login.take_if(|login| login.worker == worker) {
            self.attempt_failed(&login, now);
            if self.gives_the_screen_back(&login) {
                actions.push(Action::ShowGreeter);
            }
            let description = match login.phase {
                Phase::Starting => "the session ended before it was shown",
                Phase::Authenticating | Phase::Authenticated => "the session worker ended",
            };
            actions.extend(self.respond(Response::error(ErrorType::Error, description)));
        } else if let Some(index) = self
            .abandoned
            .iter()
            .position(|login| login.worker == worker)
        {
            let login = self.abandoned.remove(index);
            if self.gives_the_screen_back(&login) {
                actions.push(Action::ShowGreeter);
            }
        } else {
            self.sessions.retain(|session| *session != worker);
        }
        actions.extend(self.tick(now));
        actions
    }

    /// When [`Self::tick`] has something to do although nothing else happens.
    pub fn wake_at(&self) -> Option<Instant> {
        let respawn = self.respawn_at.filter(|_| self.needs_greeter());
        respawn.into_iter().chain(self.exit_by).min()
    }

    /// Does what is due at `now`.
    pub fn tick(&mut self, now: Instant) -> Vec<Action> {
        if self.needs_greeter() && self.respawn_at.is_none_or(|at| now >= at) {
            self.respawn_at = None;
            return vec![self.spawn_greeter()];
        }
        if self.exit_by.is_some_and(|at| now >= at) {
            self.exit_by = None;
            return vec![Action::Exit { success: false }];
        }
        self.exit_when_finished().into_iter().collect()
    }

    fn exit_when_finished(&self) -> Option<Action> {
        (self.ending && self.finished()).then_some(Action::Exit {
            success: !self.failed,
        })
    }

    /// Ends every session, then the daemon.
    pub fn terminate(&mut self, now: Instant) -> Vec<Action> {
        self.ending = true;
        let mut actions = self.drop_conversation(now);
        if let Some(greeter) = self.greeter.as_mut() {
            greeter.stopping = true;
            actions.push(Action::Stop(greeter.worker));
        }
        if let Some(login) = &self.login {
            actions.push(Action::Stop(login.worker));
        }
        actions.extend(self.sessions.iter().map(|session| Action::Stop(*session)));
        actions.extend(self.exit_when_finished());
        actions
    }
}
