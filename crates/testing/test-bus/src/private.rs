use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt;

use tempfile::TempDir;

use crate::SessionBus;

const LIVE: &str = "--live";

/// A private session for a benchmark: its own session bus and a fresh temporary home holding
/// every XDG base directory, XDG_RUNTIME_DIR and GNUPGHOME, as `ci/hermetic.sh` gives tests, so
/// nothing the benchmark starts reaches the person's own files, settings or services.
pub struct PrivateSession {
    _bus: Option<SessionBus>,
    _root: Option<TempDir>,
}

impl PrivateSession {
    /// Points this process, and everything it starts, at a private session, unless `--live` is
    /// among the arguments.
    ///
    /// # Safety
    ///
    /// Changes the process environment: call it before any other thread starts.
    pub unsafe fn enter() -> io::Result<Self> {
        if std::env::args().any(|argument| argument == LIVE) {
            return Ok(Self {
                _bus: None,
                _root: None,
            });
        }
        let root = tempfile::Builder::new().prefix("bande-bench.").tempdir()?;
        let home = root.path().join("home");
        let directories = [
            ("XDG_CONFIG_HOME", home.join(".config")),
            ("XDG_DATA_HOME", home.join(".local/share")),
            ("XDG_STATE_HOME", home.join(".local/state")),
            ("XDG_CACHE_HOME", home.join(".cache")),
        ];
        for (_, directory) in &directories {
            fs::create_dir_all(directory)?;
        }
        let mut private = DirBuilder::new();
        private.mode(0o700);
        let runtime = root.path().join("runtime");
        let gnupg = home.join(".gnupg");
        private.create(&runtime)?;
        private.create(&gnupg)?;
        // SAFETY: the caller runs this before any other thread starts, so nothing reads the
        // environment while it changes.
        unsafe {
            std::env::set_var("HOME", &home);
            for (variable, directory) in &directories {
                std::env::set_var(variable, directory);
            }
            std::env::set_var("XDG_RUNTIME_DIR", &runtime);
            std::env::set_var("GNUPGHOME", &gnupg);
            for variable in ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY"] {
                std::env::remove_var(variable);
            }
        }
        let bus = SessionBus::start()?;
        // SAFETY: as above; the bus is a child process, not a thread.
        unsafe {
            std::env::set_var("DBUS_SESSION_BUS_ADDRESS", bus.address());
        }
        Ok(Self {
            _bus: Some(bus),
            _root: Some(root),
        })
    }

    /// The arguments after the program's name, without `--live`.
    pub fn arguments() -> impl Iterator<Item = String> {
        std::env::args().skip(1).filter(|argument| argument != LIVE)
    }
}
