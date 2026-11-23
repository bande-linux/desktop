use std::collections::{HashMap, VecDeque};
use std::fs::Permissions;
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use bande_login::manager::{Action, GREETER_USER, Manager, WorkerId};
use bande_login::protocol::{Class, FromWorker, Request, ToWorker, read_message, write_message};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::process::{Pid, PidfdFlags, pidfd_open};

use crate::GREETER_SOCKET;
use crate::account::Account;
use crate::vt::{self, GREETER_VT, Terminal};

const OWN_PROGRAM: &str = "/proc/self/exe";
const GREETER_PATIENCE: Duration = Duration::from_secs(1);
const STOPPING_SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

struct Worker {
    child: Child,
    exited: OwnedFd,
    socket: Option<UnixStream>,
}

#[derive(Clone, Copy)]
enum Source {
    Signals,
    Listener,
    Greeter,
    Said(WorkerId),
    Exited(WorkerId),
}

struct Daemon {
    manager: Manager,
    workers: HashMap<WorkerId, Worker>,
    listener: UnixListener,
    greeter: Option<UnixStream>,
    greeter_uid: u32,
    signals: OwnedFd,
    _greeter_vt: Terminal,
    started: Instant,
    pending: VecDeque<Action>,
    exit: Option<bool>,
}

fn listen(account: &Account) -> io::Result<UnixListener> {
    match std::fs::remove_file(GREETER_SOCKET) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let listener = UnixListener::bind(GREETER_SOCKET)?;
    std::os::unix::fs::chown(GREETER_SOCKET, Some(account.uid), Some(account.gid))?;
    std::fs::set_permissions(GREETER_SOCKET, Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn stopping_signals() -> io::Result<OwnedFd> {
    // SAFETY: the set is initialised by sigemptyset before use, and signalfd returns a new
    // descriptor this function owns.
    unsafe {
        let mut set = MaybeUninit::<libc::sigset_t>::uninit();
        libc::sigemptyset(set.as_mut_ptr());
        for signal in STOPPING_SIGNALS {
            libc::sigaddset(set.as_mut_ptr(), signal);
        }
        let set = set.assume_init();
        if libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) == -1 {
            return Err(io::Error::last_os_error());
        }
        match libc::signalfd(-1, &set, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK) {
            -1 => Err(io::Error::last_os_error()),
            fd => Ok(OwnedFd::from_raw_fd(fd)),
        }
    }
}

fn start_worker(class: Class, username: String) -> io::Result<Worker> {
    let (ours, theirs) = UnixStream::pair()?;
    let child = Command::new(OWN_PROGRAM)
        .arg("--worker")
        .stdin(Stdio::from(OwnedFd::from(theirs)))
        .spawn()?;
    let exited = pidfd_open(Pid::from_child(&child), PidfdFlags::empty())?;
    write_message(&mut &ours, &ToWorker::Begin { class, username })?;
    Ok(Worker {
        child,
        exited,
        socket: Some(ours),
    })
}

/// Runs the login manager until it is stopped or the login window keeps failing.
pub fn run() -> ExitCode {
    let started = Instant::now();
    let prepared = Account::named(GREETER_USER)
        .map_err(|error| format!("cannot look up {GREETER_USER}: {error}"))
        .and_then(|account| {
            let listener = listen(&account)
                .map_err(|error| format!("cannot listen on {GREETER_SOCKET}: {error}"))?;
            let signals =
                stopping_signals().map_err(|error| format!("cannot watch for signals: {error}"))?;
            let greeter_vt = Terminal::open(GREETER_VT)
                .map_err(|error| format!("cannot open tty{GREETER_VT}: {error}"))?;
            Ok((account.uid, listener, signals, greeter_vt))
        });
    let (greeter_uid, listener, signals, greeter_vt) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let mut daemon = Daemon {
        manager: Manager::default(),
        workers: HashMap::new(),
        listener,
        greeter: None,
        greeter_uid,
        signals,
        _greeter_vt: greeter_vt,
        started,
        pending: VecDeque::new(),
        exit: None,
    };
    let actions = daemon.manager.start(daemon.now());
    daemon.act(actions);
    loop {
        if let Some(success) = daemon.exit {
            return match success {
                true => ExitCode::SUCCESS,
                false => {
                    tracing::error!("the login window keeps failing to start");
                    ExitCode::FAILURE
                }
            };
        }
        if let Err(error) = daemon.wait() {
            tracing::error!("cannot wait for the login window or the sessions: {error}");
            return ExitCode::FAILURE;
        }
    }
}

impl Daemon {
    fn now(&self) -> Duration {
        self.started.elapsed()
    }

    fn sources(&self) -> Vec<(Source, BorrowedFd<'_>)> {
        let mut sources = vec![
            (Source::Signals, self.signals.as_fd()),
            (Source::Listener, self.listener.as_fd()),
        ];
        if let Some(greeter) = &self.greeter {
            sources.push((Source::Greeter, greeter.as_fd()));
        }
        for (id, worker) in &self.workers {
            if let Some(socket) = &worker.socket {
                sources.push((Source::Said(*id), socket.as_fd()));
            }
            sources.push((Source::Exited(*id), worker.exited.as_fd()));
        }
        sources
    }

    fn wait(&mut self) -> io::Result<()> {
        let sources = self.sources();
        let mut fds: Vec<PollFd<'_>> = sources
            .iter()
            .map(|(_, fd)| PollFd::from_borrowed_fd(*fd, PollFlags::IN))
            .collect();
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        let ready: Vec<Source> = sources
            .iter()
            .zip(&fds)
            .filter(|(_, fd)| !fd.revents().is_empty())
            .map(|((source, _), _)| *source)
            .collect();
        drop(fds);
        drop(sources);
        for source in ready {
            let actions = match source {
                Source::Signals => self.signalled(),
                Source::Listener => self.accept(),
                Source::Greeter => self.hear_greeter(),
                Source::Said(worker) => self.hear_worker(worker),
                Source::Exited(worker) => self.reap(worker),
            };
            self.act(actions);
        }
        Ok(())
    }

    fn signalled(&mut self) -> Vec<Action> {
        let mut info = [0u8; std::mem::size_of::<libc::signalfd_siginfo>()];
        match rustix::io::read(&self.signals, &mut info) {
            Ok(_) => {
                tracing::info!("stopping: ending every session");
                self.manager.terminate()
            }
            Err(rustix::io::Errno::AGAIN) => Vec::new(),
            Err(error) => {
                tracing::warn!("cannot read the signal: {error}");
                Vec::new()
            }
        }
    }

    fn accept(&mut self) -> Vec<Action> {
        let stream = match self.listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) => {
                tracing::warn!("cannot accept the login window: {error}");
                return Vec::new();
            }
        };
        match rustix::net::sockopt::socket_peercred(&stream) {
            Ok(credentials) if credentials.uid.as_raw() == self.greeter_uid => {}
            Ok(credentials) => {
                tracing::warn!(
                    "refused a connection from uid {}, not the login window",
                    credentials.uid.as_raw()
                );
                return Vec::new();
            }
            Err(error) => {
                tracing::warn!("cannot tell who connected: {error}");
                return Vec::new();
            }
        }
        let patient = stream
            .set_read_timeout(Some(GREETER_PATIENCE))
            .and_then(|()| stream.set_write_timeout(Some(GREETER_PATIENCE)));
        if let Err(error) = patient {
            tracing::warn!("cannot bound the login window's messages: {error}");
            return Vec::new();
        }
        match self.greeter.replace(stream) {
            Some(_) => self.manager.disconnected(),
            None => Vec::new(),
        }
    }

    fn forget_greeter(&mut self) -> Vec<Action> {
        self.greeter = None;
        self.manager.disconnected()
    }

    fn hear_greeter(&mut self) -> Vec<Action> {
        let Some(greeter) = self.greeter.as_mut() else {
            return Vec::new();
        };
        match read_message::<Request>(greeter) {
            Ok(request) => self.manager.request(request),
            Err(error) => {
                if error.kind() != io::ErrorKind::UnexpectedEof {
                    tracing::warn!("dropping the login window's connection: {error}");
                }
                self.forget_greeter()
            }
        }
    }

    fn hear_worker(&mut self, id: WorkerId) -> Vec<Action> {
        let Some(worker) = self.workers.get_mut(&id) else {
            return Vec::new();
        };
        let Some(socket) = worker.socket.as_mut() else {
            return Vec::new();
        };
        match read_message::<FromWorker>(socket) {
            Ok(said) => self.manager.worker_said(id, said),
            Err(_) => {
                worker.socket = None;
                Vec::new()
            }
        }
    }

    fn reap(&mut self, id: WorkerId) -> Vec<Action> {
        let Some(mut worker) = self.workers.remove(&id) else {
            return Vec::new();
        };
        match worker.child.wait() {
            Ok(status) if status.success() => {}
            Ok(status) => tracing::info!("session worker {id} ended: {status}"),
            Err(error) => tracing::warn!("cannot reap session worker {id}: {error}"),
        }
        self.manager.worker_ended(id, self.now())
    }

    fn act(&mut self, actions: Vec<Action>) {
        self.pending.extend(actions);
        while let Some(action) = self.pending.pop_front() {
            let follow = self.perform(action);
            self.pending.extend(follow);
        }
    }

    fn perform(&mut self, action: Action) -> Vec<Action> {
        match action {
            Action::Spawn {
                worker,
                class,
                username,
            } => match start_worker(class, username) {
                Ok(started) => {
                    self.workers.insert(worker, started);
                    Vec::new()
                }
                Err(error) => {
                    tracing::error!("cannot start a session worker: {error}");
                    self.manager.worker_ended(worker, self.now())
                }
            },
            Action::Tell(id, message) => {
                let told = self
                    .workers
                    .get(&id)
                    .and_then(|worker| worker.socket.as_ref())
                    .map(|socket| write_message(&mut &*socket, &message));
                if let Some(Err(error)) = told {
                    tracing::warn!("cannot reach session worker {id}: {error}");
                }
                Vec::new()
            }
            Action::Stop(id) => {
                if let Some(worker) = self.workers.get_mut(&id) {
                    worker.socket = None;
                }
                Vec::new()
            }
            Action::Kill(id) => {
                if let Some(worker) = self.workers.get_mut(&id)
                    && let Err(error) = worker.child.kill()
                {
                    tracing::warn!("cannot end session worker {id}: {error}");
                }
                Vec::new()
            }
            Action::Reply(response) => match self.greeter.as_mut() {
                Some(greeter) => match write_message(greeter, &response) {
                    Ok(()) => Vec::new(),
                    Err(error) => {
                        tracing::warn!("cannot answer the login window: {error}");
                        self.forget_greeter()
                    }
                },
                None => Vec::new(),
            },
            Action::ShowGreeter => {
                if let Err(error) = vt::activate(GREETER_VT) {
                    tracing::warn!("cannot bring the login window to the front: {error}");
                }
                Vec::new()
            }
            Action::Exit { success } => {
                self.exit = Some(success);
                Vec::new()
            }
        }
    }
}
