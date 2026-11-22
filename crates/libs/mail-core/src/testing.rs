//! Debian's Dovecot on a free loopback port, run as the current user, for tests.

use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::net::{Security, Server};

const DOVECOT: &str = "/usr/sbin/dovecot";
const WAIT: Duration = Duration::from_secs(20);
/// The one user the test server accepts.
pub const USER: &str = "ann@bande.test";
/// The password of [`USER`].
pub const PASSWORD: &str = "dovecot test";

/// A running test server, stopped when dropped.
pub struct Dovecot {
    folder: tempfile::TempDir,
    port: u16,
}

/// A server on this computer without TLS, which only loopback addresses may use.
pub fn local(port: u16) -> Server {
    Server {
        host: "127.0.0.1".to_owned(),
        port,
        security: Security::Plain,
    }
}

impl Dovecot {
    /// Whether Dovecot is installed, so a test can skip itself when it is not.
    pub fn installed() -> bool {
        Path::new(DOVECOT).exists()
    }

    /// Starts a server with one maildir user and a Sent mailbox.
    pub fn start() -> io::Result<Self> {
        let folder = tempfile::Builder::new().prefix("dc").tempdir_in("/tmp")?;
        let port = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let user = std::env::var("USER").unwrap_or_else(|_| "root".to_owned());
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let dir = folder.path().display();
        let config = format!(
            "dovecot_config_version = 2.4.1\ndovecot_storage_version = 2.4.1\n\
             base_dir = {dir}/run\nstate_dir = {dir}/state\nlog_path = {dir}/log\n\
             default_internal_user = {user}\ndefault_internal_group = {user}\n\
             default_login_user = {user}\nprotocols = imap\nlisten = 127.0.0.1\nssl = no\n\
             auth_allow_cleartext = yes\nmail_driver = maildir\nmail_home = {dir}/mail\n\
             mail_path = ~/Maildir\npassdb static {{\n password = {PASSWORD}\n}}\n\
             userdb static {{\n fields {{\n  uid = {uid}\n  gid = {gid}\n }}\n}}\n\
             service imap-login {{\n chroot =\n inet_listener imap {{\n  port = {port}\n }}\n\
             inet_listener imaps {{\n  port = 0\n }}\n}}\n\
             service anvil {{\n chroot =\n}}\nnamespace inbox {{\n inbox = yes\n\
             mailbox Sent {{\n  special_use = \\Sent\n  auto = subscribe\n }}\n\
             mailbox Trash {{\n  special_use = \\Trash\n  auto = subscribe\n }}\n}}\n"
        );
        std::fs::write(folder.path().join("dovecot.conf"), config)?;
        let dovecot = Self { folder, port };
        if !dovecot.command(DOVECOT, &[]).status()?.success() {
            return Err(io::Error::other("dovecot did not start"));
        }
        let deadline = Instant::now() + WAIT;
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            if Instant::now() > deadline {
                return Err(io::Error::other("dovecot never listened"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(dovecot)
    }

    /// The server's port on 127.0.0.1.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// A private folder that is removed with the server.
    pub fn folder(&self) -> &Path {
        self.folder.path()
    }

    /// A Dovecot tool, such as `doveadm`, set up for this server.
    pub fn command(&self, program: &str, arguments: &[&str]) -> Command {
        let mut command = Command::new(program);
        command
            .arg("-c")
            .arg(self.folder.path().join("dovecot.conf"))
            .args(arguments);
        command
    }
}

impl Drop for Dovecot {
    fn drop(&mut self) {
        let _ = self.command(DOVECOT, &["stop"]).status();
    }
}
