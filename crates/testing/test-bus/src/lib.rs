pub mod atspi;
pub mod drives;
pub mod packagekit;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

const CONFIG: &str = include_str!("../data/session.conf");

pub struct SessionBus {
    daemon: Child,
    address: String,
}

impl SessionBus {
    pub fn start() -> std::io::Result<Self> {
        let mut config = tempfile::NamedTempFile::new()?;
        config.write_all(CONFIG.as_bytes())?;
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.path().display()))
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .stdout(Stdio::piped())
            .spawn()?;
        let mut address = String::new();
        if let Some(stdout) = daemon.stdout.take() {
            BufReader::new(stdout).read_line(&mut address)?;
        }
        let address = address.trim().to_owned();
        if address.is_empty() {
            let _ = daemon.kill();
            return Err(std::io::Error::other("dbus-daemon printed no address"));
        }
        Ok(Self { daemon, address })
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn connect(&self) -> zbus::Result<zbus::blocking::Connection> {
        zbus::blocking::connection::Builder::address(self.address.as_str())?.build()
    }
}

impl Drop for SessionBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}
