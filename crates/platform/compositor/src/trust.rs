use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::thread;

use rustix::fs::inotify;
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};
use smithay::reexports::wayland_server::Client;
use smithay::reexports::wayland_server::backend::DisconnectReason;
use zbus::blocking::Connection;
use zbus::zvariant::Value;

use crate::L10N;
use crate::session::SessionRole;
use crate::state::{ClientState, State};

const CONSENT_DIRECTORY: &str = "/var/lib/bande/consent";
const ASKED_FILE: &str = "bande/consent-asked";
const SETTINGS: &str = "bande-settings";
const PRIVACY_PANE: &str = "privacy-security";
const NOTICE_TIMEOUT: i32 = 0;
const INTERPRETERS: &[&str] = &[
    "bash", "dash", "env", "fish", "java", "lua", "node", "perl", "python", "python3", "ruby",
    "sh", "zsh",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    ReadKeys,
    TypeKeys,
}

impl Access {
    fn name(self) -> &'static str {
        match self {
            Self::ReadKeys => "read-keys",
            Self::TypeKeys => "type-keys",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Claimant {
    Program(PathBuf),
    Sandboxed(String),
}

impl Claimant {
    fn shown_name(&self) -> String {
        match self {
            Self::Program(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            ),
            Self::Sandboxed(app_id) => app_id.clone(),
        }
    }

    fn consent_line(&self, access: Access) -> String {
        match self {
            Self::Program(path) => format!("{} program {}", access.name(), path.display()),
            Self::Sandboxed(app_id) => format!("{} sandboxed {app_id}", access.name()),
        }
    }
}

pub(crate) fn default_consent_file() -> PathBuf {
    Path::new(CONSENT_DIRECTORY).join(rustix::process::getuid().as_raw().to_string())
}

pub(crate) fn verified_program(pid: i32) -> Option<Claimant> {
    let process = PathBuf::from(format!("/proc/{pid}"));
    let same_namespace =
        fs::read_link(process.join("ns/mnt")).ok()? == fs::read_link("/proc/self/ns/mnt").ok()?;
    let path = fs::read_link(process.join("exe")).ok()?;
    let running = fs::metadata(process.join("exe")).ok()?;
    let installed = fs::metadata(&path).ok()?;
    let name = path.file_name()?.to_str()?;
    let trustworthy = same_namespace
        && path.starts_with("/usr")
        && running.dev() == installed.dev()
        && running.ino() == installed.ino()
        && installed.uid() == 0
        && installed.mode() & 0o022 == 0
        && !INTERPRETERS
            .iter()
            .any(|interpreter| name.split('.').next() == Some(interpreter));
    trustworthy.then_some(Claimant::Program(path))
}

pub(crate) fn is_sandboxed(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|state| state.sandboxed)
}

pub(crate) fn is_lock_screen(client: &Client) -> bool {
    client
        .get_data::<ClientState>()
        .is_some_and(|state| state.role == Some(SessionRole::LockScreen))
}

fn already_asked(line: &str) -> bool {
    let Some(file) = bande_xdg::state_home().map(|state| state.join(ASKED_FILE)) else {
        return true;
    };
    if fs::read_to_string(&file).is_ok_and(|asked| asked.lines().any(|known| known == line)) {
        return true;
    }
    let recorded = file
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::OpenOptions::new().create(true).append(true).open(&file))
        .and_then(|mut asked| writeln!(asked, "{line}"));
    if let Err(error) = recorded {
        tracing::warn!("cannot remember who asked for consent: {error}");
    }
    false
}

fn tell_person(session: Connection, access: Access, program: String) {
    thread::spawn(move || {
        let Ok(notifications) = zbus::blocking::Proxy::new(
            &session,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        ) else {
            return;
        };
        let Ok(invoked) = notifications.receive_signal("ActionInvoked") else {
            return;
        };
        let message = match access {
            Access::ReadKeys => "consent-read-keys",
            Access::TypeKeys => "consent-type-keys",
        };
        let body = format!(
            "{}\n{}",
            L10N.tr_args(message, [("program", program.as_str().into())]),
            L10N.tr("consent-where")
        );
        let open = L10N.tr("consent-open");
        let hints: HashMap<&str, Value<'_>> = HashMap::new();
        let shown: zbus::Result<u32> = notifications.call(
            "Notify",
            &(
                L10N.tr("consent-title"),
                0u32,
                "changes-prevent-symbolic",
                L10N.tr("consent-title"),
                body,
                vec!["default", open.as_str()],
                hints,
                NOTICE_TIMEOUT,
            ),
        );
        let Ok(id) = shown else {
            return;
        };
        for signal in invoked {
            let Ok((invoked_id, _)) = signal.body().deserialize::<(u32, String)>() else {
                continue;
            };
            if invoked_id == id {
                let command = [SETTINGS.to_owned(), PRIVACY_PANE.to_owned()];
                if let Err(error) = bande_xdg::spawn_detached(SETTINGS, &command, None, None) {
                    tracing::warn!("cannot open Privacy & Security: {error}");
                }
                return;
            }
        }
    });
}

impl State {
    fn consented(&self, access: Access, client: &Client) -> bool {
        let Some(claimant) = client
            .get_data::<ClientState>()
            .and_then(|state| state.claimant.as_ref())
        else {
            return false;
        };
        let wanted = claimant.consent_line(access);
        fs::read_to_string(&self.consent_file)
            .is_ok_and(|granted| granted.lines().any(|line| line.trim() == wanted))
    }

    pub(crate) fn may(&mut self, access: Access, client: &Client) -> bool {
        if self.consented(access, client) {
            self.consenting.push((client.clone(), access));
            return true;
        }
        self.ask_person(access, client);
        false
    }

    fn ask_person(&self, access: Access, client: &Client) {
        let Some(session) = self.assistive.session_bus.clone() else {
            return;
        };
        let Some(claimant) = client
            .get_data::<ClientState>()
            .and_then(|state| state.claimant.as_ref())
        else {
            return;
        };
        if !already_asked(&claimant.consent_line(access)) {
            tell_person(session, access, claimant.shown_name());
        }
    }

    fn enforce_consents(&mut self) {
        let consenting = std::mem::take(&mut self.consenting);
        for (client, access) in consenting {
            if self.consented(access, &client) {
                self.consenting.push((client, access));
            } else {
                self.display_handle
                    .backend_handle()
                    .kill_client(client.id(), DisconnectReason::ConnectionClosed);
                if access == Access::ReadKeys {
                    self.release_input_method_grab();
                }
            }
        }
    }

    pub(crate) fn watch_consents(&mut self) {
        if let Some(previous) = self.consent_watch.take() {
            self.blockers.remove(previous);
        }
        let Some(directory) = self.consent_file.parent() else {
            return;
        };
        let watched = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)
            .and_then(|watch| {
                inotify::add_watch(
                    &watch,
                    directory,
                    inotify::WatchFlags::CLOSE_WRITE
                        | inotify::WatchFlags::MOVED_TO
                        | inotify::WatchFlags::DELETE,
                )
                .map(|_| watch)
            });
        let watch = match watched {
            Ok(watch) => watch,
            Err(error) => {
                tracing::debug!("not following consent changes: {error}");
                return;
            }
        };
        let source = Generic::new(watch, Interest::READ, Mode::Level);
        match self.blockers.insert_source(source, |_, watch, state| {
            let mut events = [0u8; 4096];
            let _ = rustix::io::read(&**watch, &mut events);
            state.enforce_consents();
            Ok(PostAction::Continue)
        }) {
            Ok(token) => self.consent_watch = Some(token),
            Err(error) => tracing::warn!("cannot follow consent changes: {}", error.error),
        }
    }
}
