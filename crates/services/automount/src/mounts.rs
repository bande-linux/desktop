use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use async_io::block_on;
use bande_keyring::Keyring;
use bande_notify::Urgency;
use bande_notify::client::{self, Request};
use bande_packages::{Client, Info, Mode};
use bande_volumes::{
    MOUNTS_PATH, MOUNTS_SERVICE, SavedServer as Saved, connected_roots, load_saved_servers,
    mounted, store_saved_servers, system_mounts_root, user_mounts_root,
};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, Signal, kill_process};
use zbus::blocking::{Connection, MessageIterator};
use zbus::object_server::SignalEmitter;
use zbus::{MatchRule, message};

use crate::L10N;
use crate::askpass::{Asked, Askpass, SOCKET_VARIABLE};
use crate::discovery::Discovery;
use crate::mounts_bus::MountsInterface;
use crate::server::{Protocol, Server};

const HELPER: &str = "/usr/libexec/bande-mount-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const SSHFS: &str = "/usr/bin/sshfs";
const CIFS: &str = "/usr/sbin/mount.cifs";
const SMBCLIENT: &str = "/usr/bin/smbclient";
const DAVFS: &str = "/usr/sbin/mount.davfs";
const FUSERMOUNT: &str = "/usr/bin/fusermount3";
const KEYRING_APP: &str = "bande-automount";
const CONNECT_LIMIT: Duration = Duration::from_secs(40);
const UNMOUNT_LIMIT: Duration = Duration::from_secs(40);
const SSHFS_OPTIONS: &str = "reconnect,ServerAliveInterval=5,ServerAliveCountMax=3,ConnectTimeout=10,idmap=user,NumberOfPasswordPrompts=1,StrictHostKeyChecking=ask,nosuid,nodev";
const NM: &str = "org.freedesktop.NetworkManager";
const NM_CONNECTED_LOCAL: u32 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Connected(PathBuf),
    NeedsPassword { user: String, refused: bool },
    Shares(Vec<String>),
    HostKey(String),
    HostKeyChanged,
    Unreachable,
    NoSuchShare,
    Missing(Vec<&'static str>),
    Invalid,
    Failed(String),
}

impl Outcome {
    pub fn wire(self) -> (String, Vec<String>) {
        let (status, details) = match self {
            Self::Connected(path) => ("connected", vec![path.display().to_string()]),
            Self::NeedsPassword { user, refused } => (
                match refused {
                    true => "refused",
                    false => "password",
                },
                vec![user],
            ),
            Self::Shares(shares) => ("shares", shares),
            Self::HostKey(key) => ("host-key", vec![key]),
            Self::HostKeyChanged => ("host-key-changed", Vec::new()),
            Self::Unreachable => ("unreachable", Vec::new()),
            Self::NoSuchShare => ("no-share", Vec::new()),
            Self::Missing(packages) => {
                ("missing", packages.into_iter().map(str::to_owned).collect())
            }
            Self::Invalid => ("invalid", Vec::new()),
            Self::Failed(text) => ("failed", vec![text]),
        };
        (status.to_owned(), details)
    }
}

#[derive(Debug, Default, Clone)]
pub struct ConnectRequest {
    pub url: String,
    pub user: String,
    pub password: String,
    pub remember: bool,
    pub reconnect: bool,
    pub trusted_key: String,
}

fn classify(text: &str, user: &str) -> Outcome {
    let lower = text.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    if has(&["host identification has changed", "offending"]) {
        return Outcome::HostKeyChanged;
    }
    if has(&[
        "permission denied",
        "logon_failure",
        "access_denied",
        "could not authenticate",
        "unauthorized",
        "401",
        "authentication",
    ]) {
        return Outcome::NeedsPassword {
            user: user.to_owned(),
            refused: !user.is_empty(),
        };
    }
    if has(&[
        "no route to host",
        "could not resolve",
        "name or service not known",
        "unable to find suitable address",
        "connection timed out",
        "connection refused",
        "host is down",
        "network is unreachable",
        "operation in progress",
        "could not connect",
        "timed out",
        "bad_network_name",
        "host_unreachable",
    ]) {
        return match has(&["bad_network_name"]) {
            true => Outcome::NoSuchShare,
            false => Outcome::Unreachable,
        };
    }
    if has(&["no such file or directory", "no such device"]) {
        return Outcome::NoSuchShare;
    }
    Outcome::Failed(
        text.lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or_default()
            .to_owned(),
    )
}

fn run_bounded(
    mut command: Command,
    input: Option<String>,
    limit: Duration,
) -> std::io::Result<Option<(bool, String, String)>> {
    command
        .stdin(match input {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(input.as_bytes());
    }
    let pid = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
    let (done, finished) = mpsc::channel();
    thread::Builder::new()
        .name("mount tool".to_owned())
        .spawn(move || {
            let _ = done.send(child.wait_with_output());
        })?;
    match finished.recv_timeout(limit) {
        Ok(output) => {
            let output = output?;
            Ok(Some((
                output.status.success(),
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            )))
        }
        Err(_) => {
            if let Some(pid) = pid {
                let _ = kill_process(pid, Signal::KILL);
            }
            Ok(None)
        }
    }
}

fn mount_table() -> String {
    fs::read_to_string("/proc/self/mountinfo").unwrap_or_default()
}

fn is_mounted(point: &Path) -> bool {
    mounted(&mount_table()).any(|mount| mount.point == point)
}

pub(crate) fn free_name(name: &str) -> String {
    let taken = |candidate: &str| {
        connected_roots()
            .iter()
            .any(|root| root.join(candidate).exists())
    };
    let name = match name.is_empty() {
        true => "Server",
        false => name,
    };
    let mut candidate = name.to_owned();
    let mut number = 2;
    while taken(&candidate) {
        candidate = format!("{name} {number}");
        number += 1;
    }
    candidate
}

struct Daemon {
    pid: Option<Pid>,
    stopping: bool,
    _askpass: Option<Askpass>,
}

#[derive(Clone)]
pub struct Mounts {
    session: Connection,
    daemons: Arc<Mutex<HashMap<PathBuf, Daemon>>>,
    saved: Arc<Mutex<Vec<Saved>>>,
}

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn clear_fuse_mount(point: &Path) {
    let lazily = Command::new(FUSERMOUNT)
        .arg("-uz")
        .arg(point)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Err(error) = lazily {
        tracing::warn!("cannot unmount {}: {error}", point.display());
    }
    let _ = fs::remove_dir(point);
}

fn answers(point: &Path, deadline: Instant) -> Result<(), String> {
    let (done, finished) = mpsc::channel();
    let checked = point.to_owned();
    thread::Builder::new()
        .name("mount check".to_owned())
        .spawn(move || {
            let _ = done.send(fs::metadata(&checked).map(drop));
        })
        .map_err(|error| error.to_string())?;
    match finished.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("timed out".to_owned()),
    }
}

fn read_ready(stderr: &mut impl Read, text: &mut String) -> bool {
    let mut buffer = [0u8; 1024];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => return false,
            Ok(read) => text.push_str(&String::from_utf8_lossy(&buffer[..read])),
            Err(error) if error.kind() == ErrorKind::WouldBlock => return true,
            Err(_) => return false,
        }
    }
}

impl Mounts {
    pub fn start(session: &Connection) -> zbus::Result<Self> {
        let mounts = Self {
            session: session.clone(),
            daemons: Arc::default(),
            saved: Arc::new(Mutex::new(load_saved_servers())),
        };
        if let Some(root) = user_mounts_root() {
            if let Err(error) = fs::create_dir_all(&root) {
                tracing::warn!("cannot create {}: {error}", root.display());
            }
            mounts.clear_abandoned(&root);
        }
        let announcing = session.clone();
        let discovery = Discovery::start(move || {
            let announced = SignalEmitter::new(announcing.inner(), MOUNTS_PATH)
                .and_then(|emitter| block_on(MountsInterface::nearby_changed(&emitter)));
            if let Err(error) = announced {
                tracing::warn!("cannot announce nearby servers: {error}");
            }
        });
        session
            .object_server()
            .at(MOUNTS_PATH, MountsInterface(mounts.clone(), discovery))?;
        session.request_name(MOUNTS_SERVICE)?;
        Ok(mounts)
    }

    fn clear_abandoned(&self, root: &Path) {
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            clear_fuse_mount(&entry.path());
        }
    }

    pub fn saved(&self) -> Vec<Saved> {
        guard(&self.saved).clone()
    }

    pub fn forget(&self, url: &str) {
        let removed = {
            let mut saved = guard(&self.saved);
            let removed: Vec<Saved> = saved.iter().filter(|e| e.url == url).cloned().collect();
            saved.retain(|entry| entry.url != url);
            store_saved_servers(&saved);
            removed
        };
        for entry in removed {
            if let Some(server) = Server::parse(&entry.url)
                && let Ok(keyring) = Keyring::open(KEYRING_APP)
                && let Err(error) = keyring.forget(&keychain_key(&server.origin(), &entry.user))
            {
                tracing::warn!("cannot forget the password for {url}: {error}");
            }
        }
    }

    pub fn connect(&self, request: &ConnectRequest) -> Outcome {
        let Some(mut server) = Server::parse(&request.url) else {
            return Outcome::Invalid;
        };
        let saved_user = guard(&self.saved)
            .iter()
            .find(|entry| Server::parse(&entry.url).is_some_and(|s| s.origin() == server.origin()))
            .map(|entry| entry.user.clone());
        let user = [Some(request.user.clone()), server.user.clone(), saved_user]
            .into_iter()
            .flatten()
            .find(|user| !user.is_empty())
            .unwrap_or_default();
        server.user = (!user.is_empty()).then(|| user.clone());
        let password = match request.password.is_empty() {
            true => stored_password(&server.origin(), &user),
            false => Some(request.password.clone()),
        };
        let outcome = match server.protocol {
            Protocol::Smb if server.path.is_empty() => {
                list_shares(&server, &user, password.as_deref())
            }
            Protocol::Smb => self.mount_cifs(&server, &user, password.as_deref()),
            Protocol::Dav | Protocol::Davs => self.mount_dav(&server, &user, password.as_deref()),
            Protocol::Sftp => self.mount_sftp(&server, password, &request.trusted_key),
        };
        if let Outcome::Connected(_) | Outcome::Shares(_) = &outcome {
            self.remember(&server, &user, request);
        }
        outcome
    }

    fn remember(&self, server: &Server, user: &str, request: &ConnectRequest) {
        let url = server.url();
        if server.protocol != Protocol::Smb || !server.path.is_empty() {
            let mut saved = guard(&self.saved);
            let reconnect = request.reconnect
                || saved
                    .iter()
                    .any(|entry| entry.url == url && entry.reconnect);
            saved.retain(|entry| entry.url != url);
            saved.insert(
                0,
                Saved {
                    url,
                    user: user.to_owned(),
                    reconnect,
                },
            );
            saved.truncate(20);
            store_saved_servers(&saved);
        }
        if request.remember && !request.password.is_empty() {
            let saved = Keyring::open(KEYRING_APP).and_then(|keyring| {
                keyring.save(
                    &keychain_key(&server.origin(), user),
                    &L10N.tr_args("keychain-label", [("server", server.origin().into())]),
                    &request.password,
                )
            });
            if let Err(error) = saved {
                tracing::warn!("cannot keep the password: {error}");
            }
        }
    }

    fn already_mounted(&self, source: &str) -> Option<PathBuf> {
        let roots = connected_roots();
        mounted(&mount_table())
            .find(|mount| {
                mount.source == source
                    && mount
                        .point
                        .parent()
                        .is_some_and(|parent| roots.iter().any(|root| root == parent))
            })
            .map(|mount| mount.point)
    }

    fn connected_at(point: PathBuf, server: &Server) -> Outcome {
        let inside = server
            .inner_path()
            .iter()
            .fold(point, |path, segment| path.join(segment));
        Outcome::Connected(inside)
    }

    fn mount_cifs(&self, server: &Server, user: &str, password: Option<&str>) -> Outcome {
        if !Path::new(CIFS).exists() {
            return Outcome::Missing(vec!["cifs-utils"]);
        }
        let Some(unc) = server.unc() else {
            return Outcome::Invalid;
        };
        if let Some(point) = self.already_mounted(&unc) {
            return Self::connected_at(point, server);
        }
        let name = free_name(&server.volume_name());
        let port = server.port.map(|port| port.to_string()).unwrap_or_default();
        let input = match (user.is_empty(), password) {
            (true, _) => None,
            (false, Some(password)) => Some(format!("{password}\n")),
            (false, None) => {
                return Outcome::NeedsPassword {
                    user: user.to_owned(),
                    refused: false,
                };
            }
        };
        let mut command = Command::new(PKEXEC);
        command.args([HELPER, "mount", "cifs", &unc, &name, user, &port]);
        match run_bounded(command, input, CONNECT_LIMIT) {
            Ok(Some((true, _, _))) => {
                let uid = rustix::process::getuid().as_raw();
                Self::connected_at(system_mounts_root(uid).join(name), server)
            }
            Ok(Some((false, _, stderr))) => classify(&stderr, user),
            Ok(None) => Outcome::Unreachable,
            Err(error) => Outcome::Failed(error.to_string()),
        }
    }

    fn mount_dav(&self, server: &Server, user: &str, password: Option<&str>) -> Outcome {
        if !Path::new(DAVFS).exists() {
            return Outcome::Missing(vec!["davfs2"]);
        }
        let url = server.dav_url();
        if let Some(point) = self.already_mounted(&url) {
            return Outcome::Connected(point);
        }
        let name = free_name(&server.volume_name());
        let input = format!("{user}\n{}\n", password.unwrap_or_default());
        let mut command = Command::new(PKEXEC);
        command.args([HELPER, "mount", "davfs", &url, &name, "", ""]);
        match run_bounded(command, Some(input), CONNECT_LIMIT) {
            Ok(Some((true, _, _))) => {
                let uid = rustix::process::getuid().as_raw();
                Outcome::Connected(system_mounts_root(uid).join(name))
            }
            Ok(Some((false, _, stderr))) => classify(&stderr, user),
            Ok(None) => Outcome::Unreachable,
            Err(error) => Outcome::Failed(error.to_string()),
        }
    }

    fn mount_sftp(&self, server: &Server, password: Option<String>, trusted_key: &str) -> Outcome {
        if !Path::new(SSHFS).exists() {
            return Outcome::Missing(vec!["sshfs"]);
        }
        let source = server.sftp_source();
        if let Some(point) = self.already_mounted(&source) {
            return Outcome::Connected(point);
        }
        let Some(root) = user_mounts_root() else {
            return Outcome::Failed("no runtime folder".to_owned());
        };
        let folder = root.parent().unwrap_or(&root);
        let had_password = password.is_some();
        let askpass = match Askpass::start(folder, password, trusted_key) {
            Ok(askpass) => askpass,
            Err(error) => return Outcome::Failed(error.to_string()),
        };
        let point = root.join(free_name(&server.volume_name()));
        let mut command = Command::new(SSHFS);
        command.arg(&source).arg(&point).arg("-f");
        command.arg("-o").arg(SSHFS_OPTIONS);
        if let Some(port) = server.port {
            command.arg("-p").arg(port.to_string());
        }
        command
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(SOCKET_VARIABLE, askpass.socket())
            .env("SSH_ASKPASS", askpass_program());
        let user = server.user.clone().unwrap_or_default();
        match self.start_daemon(command, &point, Some(askpass)) {
            Ok(()) => Outcome::Connected(point),
            Err(Started::Refused(asked)) => match asked.host_key {
                Some(key) if key != trusted_key => Outcome::HostKey(key),
                _ => Outcome::NeedsPassword {
                    refused: had_password,
                    user,
                },
            },
            Err(Started::Failed(text)) => classify(&text, &user),
        }
    }

    pub fn start_daemon(
        &self,
        mut command: Command,
        point: &Path,
        askpass: Option<Askpass>,
    ) -> Result<(), Started> {
        fs::create_dir_all(point).map_err(|error| Started::Failed(error.to_string()))?;
        let failed = |text: String| {
            let _ = fs::remove_dir(point);
            Started::Failed(text)
        };
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| failed(error.to_string()))?;
        let Some(mut stderr) = child.stderr.take() else {
            return Err(failed("no error stream".to_owned()));
        };
        let _ = rustix::io::ioctl_fionbio(&stderr, true);
        let table =
            fs::File::open("/proc/self/mountinfo").map_err(|error| failed(error.to_string()))?;
        let deadline = Instant::now() + CONNECT_LIMIT;
        let mut text = String::new();
        let mut open = true;
        while !is_mounted(point) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() || !open {
                let _ = child.kill();
                let _ = child.wait();
                clear_fuse_mount(point);
                return Err(match (left.is_zero(), &askpass) {
                    (_, Some(askpass)) if askpass.asked() != Asked::default() => {
                        Started::Refused(askpass.asked())
                    }
                    (true, _) => Started::Failed("timed out".to_owned()),
                    (false, _) => Started::Failed(text),
                });
            }
            let timeout = Timespec::try_from(left).unwrap_or(Timespec {
                tv_sec: 1,
                tv_nsec: 0,
            });
            let mut watched = [
                PollFd::new(&table, PollFlags::PRI),
                PollFd::new(&stderr, PollFlags::IN),
            ];
            let _ = poll(&mut watched, Some(&timeout));
            open = read_ready(&mut stderr, &mut text);
        }
        if let Err(reason) = answers(point, deadline) {
            read_ready(&mut stderr, &mut text);
            let _ = child.kill();
            let _ = child.wait();
            clear_fuse_mount(point);
            read_ready(&mut stderr, &mut text);
            return Err(match &askpass {
                Some(askpass) if askpass.asked() != Asked::default() => {
                    Started::Refused(askpass.asked())
                }
                _ if text.trim().is_empty() => Started::Failed(reason),
                _ => Started::Failed(text),
            });
        }
        let _ = rustix::io::ioctl_fionbio(&stderr, false);
        let pid = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
        guard(&self.daemons).insert(
            point.to_owned(),
            Daemon {
                pid,
                stopping: false,
                _askpass: askpass,
            },
        );
        let mounts = self.clone();
        let point = point.to_owned();
        let spawned = thread::Builder::new()
            .name("mount daemon".to_owned())
            .spawn(move || {
                let _ = std::io::copy(&mut stderr, &mut std::io::sink());
                let _ = child.wait();
                mounts.daemon_ended(&point);
            });
        if let Err(error) = spawned {
            tracing::warn!("cannot follow a mount daemon: {error}");
        }
        Ok(())
    }

    fn daemon_ended(&self, point: &Path) {
        let daemon = guard(&self.daemons).remove(point);
        clear_fuse_mount(point);
        if let Some(Daemon {
            stopping: false, ..
        }) = daemon
        {
            let name = point
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.notify_lost(&name);
        }
    }

    fn notify_lost(&self, name: &str) {
        let body = L10N.tr_args("connection-lost-body", [("name", name.into())]);
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &L10N.tr("app-name"),
                replaces_id: 0,
                app_icon: "folder-remote",
                summary: &L10N.tr("connection-lost"),
                body: &body,
                actions: &[],
                urgency: Urgency::Normal,
                sound: None,
            },
        );
        if let Err(error) = posted {
            tracing::warn!("cannot post the notification: {error}");
        }
    }

    pub fn notify_device_locked(&self, name: &str) {
        let body = L10N.tr_args("device-locked-body", [("name", name.into())]);
        let posted = client::notify(
            &self.session,
            &Request {
                app_name: &L10N.tr("app-name"),
                replaces_id: 0,
                app_icon: "phone",
                summary: &L10N.tr("device-locked"),
                body: &body,
                actions: &[],
                urgency: Urgency::Normal,
                sound: None,
            },
        );
        if let Err(error) = posted {
            tracing::warn!("cannot post the notification: {error}");
        }
    }

    pub fn disconnect(&self, point: &Path) -> Result<(), String> {
        let daemon = {
            let mut daemons = guard(&self.daemons);
            daemons.get_mut(point).map(|daemon| {
                daemon.stopping = true;
                daemon.pid
            })
        };
        if let Some(pid) = daemon {
            if let Some(pid) = pid {
                let _ = kill_process(pid, Signal::TERM);
            }
            clear_fuse_mount(point);
            return Ok(());
        }
        let uid = rustix::process::getuid().as_raw();
        if point.parent() != Some(system_mounts_root(uid).as_path()) {
            if user_mounts_root().as_deref() == point.parent() {
                clear_fuse_mount(point);
                return Ok(());
            }
            return Err("not a connected server".to_owned());
        }
        let name: OsString = point
            .file_name()
            .map(OsString::from)
            .ok_or_else(|| "not a connected server".to_owned())?;
        let mut command = Command::new(PKEXEC);
        command.arg(HELPER).arg("unmount").arg(name);
        match run_bounded(command, None, UNMOUNT_LIMIT) {
            Ok(Some((true, _, _))) => Ok(()),
            Ok(Some((false, _, stderr))) => Err(stderr),
            Ok(None) => Err("the server did not answer".to_owned()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn install(&self, packages: &[String]) -> Result<(), String> {
        let system = Connection::system().map_err(|error| error.to_string())?;
        let client = Client::new(&system);
        let ids: Vec<String> = client
            .resolve(packages)
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|package| package.info == Info::Available)
            .map(|package| package.id)
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        client
            .install(&ids, Mode::Apply, &|_| {})
            .map(drop)
            .map_err(|error| error.to_string())
    }

    pub fn reconnect_saved(self) {
        let spawned = thread::Builder::new()
            .name("reconnect".to_owned())
            .spawn(move || self.reconnect_loop());
        if let Err(error) = spawned {
            tracing::warn!("cannot reconnect servers: {error}");
        }
    }

    fn try_saved(&self, pending: Vec<Saved>) -> Vec<Saved> {
        pending
            .into_iter()
            .filter(|entry| {
                let request = ConnectRequest {
                    url: entry.url.clone(),
                    user: entry.user.clone(),
                    ..ConnectRequest::default()
                };
                let outcome = self.connect(&request);
                if outcome != Outcome::Unreachable && !matches!(outcome, Outcome::Connected(_)) {
                    tracing::warn!("cannot reconnect {}: {outcome:?}", entry.url);
                }
                outcome == Outcome::Unreachable
            })
            .collect()
    }

    fn reconnect_loop(&self) {
        let wanted: Vec<Saved> = self
            .saved()
            .into_iter()
            .filter(|entry| entry.reconnect)
            .collect();
        let mut pending = self.try_saved(wanted);
        if pending.is_empty() {
            return;
        }
        let Some(changes) = network_changes() else {
            return;
        };
        for change in changes.flatten() {
            let Ok((state,)) = change.body().deserialize::<(u32,)>() else {
                continue;
            };
            if state >= NM_CONNECTED_LOCAL {
                pending = self.try_saved(pending);
                if pending.is_empty() {
                    return;
                }
            }
        }
    }
}

pub enum Started {
    Refused(Asked),
    Failed(String),
}

fn network_changes() -> Option<MessageIterator> {
    let system = Connection::system()
        .inspect_err(|error| tracing::warn!("cannot follow the network: {error}"))
        .ok()?;
    let rule = MatchRule::builder()
        .msg_type(message::Type::Signal)
        .sender(NM)
        .ok()?
        .interface(NM)
        .ok()?
        .member("StateChanged")
        .ok()?
        .build();
    MessageIterator::for_match_rule(rule, &system, None)
        .inspect_err(|error| tracing::warn!("cannot follow the network: {error}"))
        .ok()
}

fn keychain_key<'a>(origin: &'a str, user: &'a str) -> [(&'a str, &'a str); 2] {
    [("server", origin), ("user", user)]
}

fn stored_password(origin: &str, user: &str) -> Option<String> {
    Keyring::open(KEYRING_APP)
        .and_then(|keyring| keyring.password(&keychain_key(origin, user)))
        .inspect_err(|error| tracing::warn!("cannot read the keychain: {error}"))
        .ok()
        .flatten()
}

fn askpass_program() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/bande-automount"))
}

fn parse_shares(listing: &str) -> Vec<String> {
    let mut shares: Vec<String> = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('|');
            (fields.next()? == "Disk").then_some(fields.next()?.to_owned())
        })
        .filter(|name| !name.ends_with('$'))
        .collect();
    shares.sort();
    shares
}

fn list_shares(server: &Server, user: &str, password: Option<&str>) -> Outcome {
    if !Path::new(SMBCLIENT).exists() {
        return Outcome::Missing(vec!["smbclient", "cifs-utils"]);
    }
    if !user.is_empty() && password.is_none() {
        return Outcome::NeedsPassword {
            user: user.to_owned(),
            refused: false,
        };
    }
    let mut command = Command::new(SMBCLIENT);
    command
        .arg("-L")
        .arg(format!("//{}", server.host))
        .arg("-g")
        .env("LC_ALL", "C");
    if let Some(port) = server.port {
        command.arg("-p").arg(port.to_string());
    }
    match (user.is_empty(), password) {
        (false, Some(password)) => {
            command.env("USER", user).env("PASSWD", password);
        }
        _ => {
            command.arg("-N");
        }
    }
    match run_bounded(command, None, CONNECT_LIMIT) {
        Ok(Some((true, stdout, _))) => Outcome::Shares(parse_shares(&stdout)),
        Ok(Some((false, stdout, stderr))) => classify(&format!("{stdout}\n{stderr}"), user),
        Ok(None) => Outcome::Unreachable,
        Err(error) => Outcome::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_errors_tell_what_the_user_can_do() {
        let refused = classify("mount error(13): Permission denied", "ann");
        assert_eq!(
            refused,
            Outcome::NeedsPassword {
                user: "ann".to_owned(),
                refused: true
            }
        );
        assert_eq!(
            classify("ssh: connect to host x: No route to host", ""),
            Outcome::Unreachable
        );
        assert_eq!(
            classify("tree connect failed: NT_STATUS_BAD_NETWORK_NAME", ""),
            Outcome::NoSuchShare
        );
        assert_eq!(
            classify("mount error(2): No such file or directory", ""),
            Outcome::NoSuchShare
        );
    }
}
