pub mod bluez;
pub mod networkmanager;
pub mod permission_store;

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

const CONFIG_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/session.conf");

pub struct SessionBus {
    daemon: Child,
    address: String,
}

impl SessionBus {
    pub fn start() -> std::io::Result<Self> {
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={CONFIG_FILE}"))
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
