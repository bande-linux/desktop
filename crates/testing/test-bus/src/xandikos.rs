use std::io;
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

const XANDIKOS: &str = "/usr/bin/xandikos";
const START: Duration = Duration::from_secs(20);

/// Debian's CalDAV and CardDAV server on a temporary folder, signed in as `Jane
/// <jane@bande.test>` without a password.
pub struct Xandikos {
    server: Child,
    _folder: TempDir,
    url: String,
}

impl Xandikos {
    /// Starts the server on a free local port, or says that it is not installed.
    pub fn start() -> io::Result<Option<Self>> {
        if !std::path::Path::new(XANDIKOS).exists() {
            eprintln!("{XANDIKOS} is not installed; skipping");
            return Ok(None);
        }
        let folder = tempfile::tempdir()?;
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let server = Command::new(XANDIKOS)
            .args([
                "serve",
                "--defaults",
                "--no-detect-systemd",
                "-l",
                "127.0.0.1",
            ])
            .arg("-p")
            .arg(port.to_string())
            .arg("-d")
            .arg(folder.path().join("dav"))
            .arg("--state-dir")
            .arg(folder.path().join("state"))
            .env("EMAIL", "Jane <jane@bande.test>")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let started = Self {
            server,
            _folder: folder,
            url: format!("http://127.0.0.1:{port}/"),
        };
        let deadline = Instant::now() + START;
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if Instant::now() >= deadline {
                return Err(io::Error::other("xandikos never listened"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(Some(started))
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for Xandikos {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
    }
}
