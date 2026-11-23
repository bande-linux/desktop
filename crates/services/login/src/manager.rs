use std::collections::VecDeque;
use std::time::Duration;

use crate::protocol::{
    AuthMessageType, Class, ErrorType, FromWorker, Request, Response, ToWorker, carried_environment,
};

/// The login window's own account.
pub const GREETER_USER: &str = "_bande-greeter";
const QUICK_END: Duration = Duration::from_secs(10);
const QUICK_ENDS_ALLOWED: u32 = 3;
const USERNAME_LIMIT: usize = 256;

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
    /// Lets go of a worker, which ends the session it opened.
    Stop(WorkerId),
    /// Ends a worker that has opened no session, in the middle of its conversation.
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
    phase: Phase,
}

#[derive(Debug)]
struct Greeter {
    worker: WorkerId,
    since: Duration,
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
    abandoned: Vec<WorkerId>,
    waiting: bool,
    asked: bool,
    queued: VecDeque<Response>,
    quick_ends: u32,
    ending: bool,
}

fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= USERNAME_LIMIT
        && !username.starts_with('-')
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

    fn spawn_greeter(&mut self, now: Duration) -> Action {
        let (worker, action) = self.spawn(Class::Greeter, GREETER_USER);
        self.greeter = Some(Greeter {
            worker,
            since: now,
            stopping: false,
        });
        action
    }

    pub fn start(&mut self, now: Duration) -> Vec<Action> {
        vec![self.spawn_greeter(now)]
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

    fn drop_conversation(&mut self) -> Vec<Action> {
        self.queued.clear();
        self.asked = false;
        match self.login.take() {
            Some(login) if login.phase == Phase::Starting => {
                self.login = Some(login);
                Vec::new()
            }
            Some(login) => {
                self.abandoned.push(login.worker);
                vec![Action::Kill(login.worker)]
            }
            None => Vec::new(),
        }
    }

    /// Handles a request from the login window.
    pub fn request(&mut self, request: Request) -> Vec<Action> {
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
                let mut actions = self.drop_conversation();
                let (worker, spawn) = self.spawn(Class::User, &username);
                self.login = Some(Login {
                    worker,
                    phase: Phase::Authenticating,
                });
                actions.push(spawn);
                actions.extend(self.await_worker());
                actions
            }
            Request::PostAuthMessageResponse { response } => {
                let Some(login) = self
                    .login
                    .as_ref()
                    .filter(|login| login.phase == Phase::Authenticating)
                else {
                    return vec![Action::Reply(Response::error(
                        ErrorType::Error,
                        "nothing was asked",
                    ))];
                };
                let worker = login.worker;
                let mut actions = Vec::new();
                if std::mem::take(&mut self.asked) {
                    actions.push(Action::Tell(worker, ToWorker::Answer { response }));
                }
                actions.extend(self.await_worker());
                actions
            }
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
                let mut actions = match self.login.take() {
                    Some(login) if login.phase == Phase::Starting => {
                        self.abandoned.push(login.worker);
                        vec![Action::Stop(login.worker)]
                    }
                    Some(login) => {
                        self.abandoned.push(login.worker);
                        vec![Action::Kill(login.worker)]
                    }
                    None => Vec::new(),
                };
                self.queued.clear();
                self.asked = false;
                actions.push(Action::Reply(Response::Success));
                self.waiting = false;
                actions
            }
        }
    }

    /// Forgets the login window's connection, abandoning a conversation that has not yet
    /// started a session.
    pub fn disconnected(&mut self) -> Vec<Action> {
        self.waiting = false;
        self.drop_conversation()
    }

    /// Handles what a worker said.
    pub fn worker_said(&mut self, worker: WorkerId, said: FromWorker) -> Vec<Action> {
        let Some(login) = self.login.as_mut().filter(|login| login.worker == worker) else {
            return Vec::new();
        };
        match (login.phase, said) {
            (Phase::Authenticating, FromWorker::Ask { text, visible }) => {
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
                let starting = login.phase == Phase::Starting;
                self.abandoned.push(worker);
                self.login = None;
                let mut actions = self.respond(Response::Error {
                    error_type,
                    description,
                });
                if starting {
                    actions.push(Action::ShowGreeter);
                }
                actions
            }
            (Phase::Starting, FromWorker::Shown) => {
                self.login = None;
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
        self.greeter.is_none() && self.login.is_none() && self.sessions.is_empty()
    }

    /// Handles a worker that has ended, at `now` on the monotonic clock.
    pub fn worker_ended(&mut self, worker: WorkerId, now: Duration) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(greeter) = self.greeter.take_if(|greeter| greeter.worker == worker) {
            let unexpected = !greeter.stopping && !self.ending;
            if unexpected {
                tracing::warn!("the login window ended");
                actions.extend(self.drop_conversation());
                match now.saturating_sub(greeter.since) < QUICK_END {
                    true => self.quick_ends += 1,
                    false => self.quick_ends = 0,
                }
                if self.quick_ends >= QUICK_ENDS_ALLOWED && self.finished() {
                    actions.push(Action::Exit { success: false });
                    return actions;
                }
            }
        } else if let Some(login) = self.login.take_if(|login| login.worker == worker) {
            let description = match login.phase {
                Phase::Starting => {
                    if !self.ending {
                        actions.push(Action::ShowGreeter);
                    }
                    "the session ended before it was shown"
                }
                _ => "the session worker ended",
            };
            actions.extend(self.respond(Response::error(ErrorType::Error, description)));
        } else if let Some(index) = self.sessions.iter().position(|session| *session == worker) {
            self.sessions.remove(index);
        } else {
            self.abandoned.retain(|abandoned| *abandoned != worker);
        }
        if self.needs_greeter() {
            actions.push(self.spawn_greeter(now));
        }
        if self.ending && self.finished() {
            actions.push(Action::Exit { success: true });
        }
        actions
    }

    /// Ends every session, then the daemon.
    pub fn terminate(&mut self) -> Vec<Action> {
        self.ending = true;
        let mut actions = self.drop_conversation();
        if let Some(greeter) = self.greeter.as_mut() {
            greeter.stopping = true;
            actions.push(Action::Stop(greeter.worker));
        }
        if let Some(login) = &self.login {
            actions.push(Action::Stop(login.worker));
        }
        actions.extend(self.sessions.iter().map(|session| Action::Stop(*session)));
        if self.finished() {
            actions.push(Action::Exit { success: true });
        }
        actions
    }
}
