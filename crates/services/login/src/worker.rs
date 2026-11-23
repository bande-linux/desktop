use std::ffi::{OsStr, OsString};
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitCode, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use bande_bus::logind::{ManagerProxyBlocking, SessionProxyBlocking};
use bande_login::protocol::{
    Class, ErrorType, FromWorker, SOCKET_VARIABLE, ToWorker, read_message, write_message,
};
use bande_login::{Relay, ShownWatch, Unattended};
use bande_pam::{Code, Conversation, DELETE_CRED, ESTABLISH_CRED, Transaction};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, PidfdFlags, Signal, pidfd_open};
use zbus::proxy::CacheProperties;

use crate::GREETER_SOCKET;
use crate::account::Account;
use crate::vt::{self, GREETER_VT, Terminal};

const USER_SERVICE: &str = "bande-greeter";
const GREETER_SERVICE: &str = "bande-greeter-session";
const GREETER_COMMAND: &str = "/usr/libexec/bande-greeter-session";
const SESSION_SCRIPT: &str = "[ -f /etc/profile ] && . /etc/profile; \
    [ -f \"$HOME/.profile\" ] && . \"$HOME/.profile\"; exec /usr/bin/bande-session";
const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin:/usr/games";
const ACTIVE_LIMIT: Duration = Duration::from_secs(3);
const ACTIVE_POLL: Duration = Duration::from_millis(10);
const STOP_LIMIT: Duration = Duration::from_secs(10);

struct Failure {
    error_type: ErrorType,
    description: String,
}

impl Failure {
    fn pam(step: &str, code: Code) -> Self {
        let error_type = match code {
            Code::AUTH_ERR | Code::CRED_INSUFFICIENT | Code::USER_UNKNOWN | Code::MAXTRIES => {
                ErrorType::AuthError
            }
            _ => ErrorType::Error,
        };
        Self {
            error_type,
            description: format!("{step}: {code}"),
        }
    }

    fn other(description: impl std::fmt::Display) -> Self {
        Self {
            error_type: ErrorType::Error,
            description: description.to_string(),
        }
    }
}

fn tell(socket: &UnixStream, said: &FromWorker) {
    if let Err(error) = write_message(&mut &*socket, said) {
        tracing::warn!("cannot reach bande-login: {error}");
    }
}

/// Runs one session worker: the conversation, the session's PAM transaction and the session's
/// processes, on the socket bande-login handed it as standard input.
pub fn run() -> ExitCode {
    // SAFETY: bande-login starts every worker with its socket as standard input, and nothing
    // else in this process owns descriptor 0.
    let socket = UnixStream::from(unsafe { OwnedFd::from_raw_fd(0) });
    let (class, username) = match read_message(&mut &socket) {
        Ok(ToWorker::Begin { class, username }) => (class, username),
        Ok(other) => {
            tracing::error!("a session worker was sent {other:?} first");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            tracing::error!("a session worker heard nothing: {error}");
            return ExitCode::FAILURE;
        }
    };
    let ended = match class {
        Class::User => serve(class, &username, &socket, &mut Relay::new(&socket)),
        Class::Greeter => serve(class, &username, &socket, &mut Unattended),
    };
    match ended {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            tracing::info!("no session for {username}: {}", failure.description);
            tell(
                &socket,
                &FromWorker::Failed {
                    error_type: failure.error_type,
                    description: failure.description,
                },
            );
            ExitCode::FAILURE
        }
    }
}

fn serve(
    class: Class,
    username: &str,
    socket: &UnixStream,
    conversation: &mut dyn Conversation,
) -> Result<(), Failure> {
    let service = match class {
        Class::User => USER_SERVICE,
        Class::Greeter => GREETER_SERVICE,
    };
    let mut pam = Transaction::start(service, username, conversation).map_err(Failure::other)?;
    if class == Class::User {
        pam.authenticate()
            .map_err(|code| Failure::pam("pam_authenticate", code))?;
    }
    match pam.check_account() {
        Ok(()) => {}
        Err(Code::NEW_AUTHTOK_REQD) if class == Class::User => pam
            .change_expired_password()
            .map_err(|code| Failure::pam("pam_chauthtok", code))?,
        Err(code) => return Err(Failure::pam("pam_acct_mgmt", code)),
    }
    pam.credentials(ESTABLISH_CRED)
        .map_err(|code| Failure::pam("pam_setcred", code))?;
    let carried = match class {
        Class::User => {
            tell(socket, &FromWorker::Authenticated);
            match read_message(&mut &*socket) {
                Ok(ToWorker::Launch { env }) => env,
                Ok(other) => return Err(Failure::other(format!("expected Launch, got {other:?}"))),
                Err(error) => {
                    tracing::info!("signing in was called off: {error}");
                    return Ok(());
                }
            }
        }
        Class::Greeter => vec![format!("{SOCKET_VARIABLE}={GREETER_SOCKET}")],
    };
    let user = pam.user().unwrap_or_else(|| username.to_owned());
    let account = Account::named(&user)
        .map_err(|error| Failure::other(format!("cannot look up {user}: {error}")))?;
    let number = match class {
        Class::User => {
            vt::unused().map_err(|error| Failure::other(format!("no terminal: {error}")))?
        }
        Class::Greeter => GREETER_VT,
    };
    let mut terminal = Terminal::open(number)
        .map_err(|error| Failure::other(format!("cannot open tty{number}: {error}")))?;
    pam.set_tty(&format!("tty{number}"))
        .map_err(|code| Failure::pam("pam_set_item", code))?;
    for assignment in session_environment(class, &account, number)
        .iter()
        .chain(&carried)
    {
        pam.put_env(assignment)
            .map_err(|code| Failure::pam("pam_putenv", code))?;
    }
    pam.open_session()
        .map_err(|code| Failure::pam("pam_open_session", code))?;
    let ran = run_session(class, &pam, &account, &mut terminal, socket);
    if let Err(code) = pam.close_session() {
        tracing::warn!("pam_close_session: {code}");
    }
    if let Err(code) = pam.credentials(DELETE_CRED) {
        tracing::warn!("pam_setcred: {code}");
    }
    ran
}

fn session_environment(class: Class, account: &Account, number: u32) -> Vec<String> {
    let text = |value: &std::ffi::CStr| value.to_string_lossy().into_owned();
    let mut environment = vec![
        "XDG_SEAT=seat0".to_owned(),
        format!("XDG_VTNR={number}"),
        "XDG_SESSION_TYPE=wayland".to_owned(),
        format!("HOME={}", text(&account.home)),
        format!("USER={}", text(&account.name)),
        format!("LOGNAME={}", text(&account.name)),
        format!("SHELL={}", text(&account.shell)),
        format!("PATH={DEFAULT_PATH}"),
    ];
    environment.extend(match class {
        Class::User => [
            "XDG_SESSION_CLASS=user",
            "XDG_SESSION_DESKTOP=bande",
            "XDG_CURRENT_DESKTOP=X-Bande",
        ]
        .map(str::to_owned)
        .to_vec(),
        Class::Greeter => vec!["XDG_SESSION_CLASS=greeter".to_owned()],
    });
    environment
}

fn variable(environment: &[(OsString, OsString)], name: &str) -> Option<OsString> {
    environment
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

fn wait_until_active(session: &str) -> zbus::Result<bool> {
    let system = bande_bus::system()?;
    let path = ManagerProxyBlocking::new(&system)?.get_session(session)?;
    let session = SessionProxyBlocking::builder(&system)
        .path(path)?
        .cache_properties(CacheProperties::No)
        .build()?;
    let deadline = Instant::now() + ACTIVE_LIMIT;
    while !session.active()? {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(ACTIVE_POLL);
    }
    Ok(true)
}

fn run_session(
    class: Class,
    pam: &Transaction<'_>,
    account: &Account,
    terminal: &mut Terminal,
    socket: &UnixStream,
) -> Result<(), Failure> {
    let environment: Vec<(OsString, OsString)> = pam
        .environment()
        .into_iter()
        .filter_map(|entry| {
            let bytes = entry.as_bytes();
            let split = bytes.iter().position(|byte| *byte == b'=')?;
            Some((
                OsStr::from_bytes(&bytes[..split]).to_owned(),
                OsStr::from_bytes(&bytes[split + 1..]).to_owned(),
            ))
        })
        .collect();
    terminal.blank();
    terminal.show().map_err(|error| {
        Failure::other(format!("cannot show tty{}: {error}", terminal.number()))
    })?;
    match variable(&environment, "XDG_SESSION_ID") {
        Some(session) => match wait_until_active(&session.to_string_lossy()) {
            Ok(true) => {}
            Ok(false) => tracing::warn!("logind has not made session {session:?} active"),
            Err(error) => tracing::warn!("cannot ask logind about the session: {error}"),
        },
        None => tracing::warn!("pam_systemd registered no session"),
    }
    let followed = launch(class, account, terminal, &environment, socket);
    if class == Class::User
        && !matches!(followed, Ok(true))
        && let Err(error) = terminal.leave_to(GREETER_VT)
    {
        tracing::warn!("cannot bring the login window back: {error}");
    }
    followed.map(|_| ())
}

fn launch(
    class: Class,
    account: &Account,
    terminal: &Terminal,
    environment: &[(OsString, OsString)],
    socket: &UnixStream,
) -> Result<bool, Failure> {
    let watch = match (class, variable(environment, "XDG_RUNTIME_DIR")) {
        (Class::User, Some(runtime)) => ShownWatch::new(runtime.as_ref())
            .inspect_err(|error| tracing::warn!("cannot wait for the desktop: {error}"))
            .ok(),
        _ => None,
    };
    let mut child = spawn(class, account, terminal, environment)
        .map_err(|error| Failure::other(format!("cannot start the session: {error}")))?;
    let mut shown = class == Class::Greeter;
    if class == Class::User && watch.is_none() {
        tell(socket, &FromWorker::Shown);
        shown = true;
    }
    let status = follow(&mut child, socket, watch.as_ref(), &mut shown)
        .map_err(|error| Failure::other(format!("cannot follow the session: {error}")))?;
    tracing::info!(
        "the session of {} ended: {status}",
        account.name.to_string_lossy()
    );
    Ok(shown)
}

fn spawn(
    class: Class,
    account: &Account,
    terminal: &Terminal,
    environment: &[(OsString, OsString)],
) -> io::Result<Child> {
    let mut command = match class {
        Class::User => {
            let mut command = Command::new("/bin/sh");
            command.arg("-c").arg(SESSION_SCRIPT);
            command
        }
        Class::Greeter => Command::new(GREETER_COMMAND),
    };
    command
        .env_clear()
        .envs(environment.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::from(terminal.fd().try_clone_to_owned()?));
    let uid = account.uid;
    let gid = account.gid;
    let groups = account.groups.clone();
    let home = account.home.clone();
    // SAFETY: the closure runs in the forked child before exec and makes only async-signal-safe
    // system calls on values prepared before the fork.
    unsafe {
        command.pre_exec(move || {
            let checked = |result: libc::c_int| match result {
                -1 => Err(io::Error::last_os_error()),
                _ => Ok(()),
            };
            checked(libc::setsid())?;
            checked(libc::ioctl(0, libc::TIOCSCTTY, 1))?;
            checked(libc::setgroups(groups.len(), groups.as_ptr()))?;
            checked(libc::setgid(gid))?;
            checked(libc::setuid(uid))?;
            checked(libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM))?;
            if libc::chdir(home.as_ptr()) == -1 {
                checked(libc::chdir(c"/".as_ptr()))?;
            }
            Ok(())
        });
    }
    command.spawn()
}

fn follow(
    child: &mut Child,
    socket: &UnixStream,
    watch: Option<&ShownWatch>,
    shown: &mut bool,
) -> io::Result<ExitStatus> {
    let pid = Pid::from_child(child);
    let exited = pidfd_open(pid, PidfdFlags::empty())?;
    let mut stopping: Option<Instant> = None;
    loop {
        let mut fds = vec![PollFd::new(&exited, PollFlags::IN)];
        if stopping.is_none() {
            fds.push(PollFd::new(socket, PollFlags::IN));
        }
        let watching = watch.filter(|_| !*shown);
        if let Some(watch) = watching {
            fds.push(PollFd::from_borrowed_fd(watch.fd(), PollFlags::IN));
        }
        let timeout = stopping
            .map(|since| STOP_LIMIT.saturating_sub(since.elapsed()))
            .map(Timespec::try_from)
            .transpose()
            .map_err(io::Error::other)?;
        match poll(&mut fds, timeout.as_ref()) {
            Ok(_) | Err(rustix::io::Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
        let exited_now = !fds[0].revents().is_empty();
        let hung_up = stopping.is_none() && fds.get(1).is_some_and(|fd| !fd.revents().is_empty());
        let marked = watching.is_some() && fds.last().is_some_and(|fd| !fd.revents().is_empty());
        if exited_now {
            return child.wait();
        }
        if marked
            && watching.is_some_and(|watch| {
                watch.is_shown().unwrap_or_else(|error| {
                    tracing::warn!("cannot look for the desktop's first frame: {error}");
                    false
                })
            })
        {
            *shown = true;
            tell(socket, &FromWorker::Shown);
        }
        if hung_up {
            tracing::info!("ending the session");
            signal_group(pid, Signal::TERM);
            stopping = Some(Instant::now());
        }
        if stopping.is_some_and(|since| since.elapsed() >= STOP_LIMIT) {
            tracing::warn!(
                "the session did not end within {} s; killing it",
                STOP_LIMIT.as_secs()
            );
            signal_group(pid, Signal::KILL);
            return child.wait();
        }
    }
}

fn signal_group(pid: Pid, signal: Signal) {
    match rustix::process::kill_process_group(pid, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => {}
        Err(error) => tracing::warn!("cannot signal the session: {error}"),
    }
}
