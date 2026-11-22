use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};

use bande_app::choose_file;
use bande_l10n::tr;
use bande_packages::Client;
use bande_toolkit::ui::{Button, SecureField, Toggle, Widget};
use zbus::blocking::Connection;

use crate::follow::{Update, Updates, spawn};
use crate::pages::{FIELD_SIZE, Widgets, boxed, submit};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const HELPER: &str = "/usr/libexec/bande-sharing-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const SAMBA: &str = "samba";
const AVAHI: &str = "avahi-daemon";
const AVAHI_DAEMON: &str = "/usr/sbin/avahi-daemon";
const SMBD: &str = "/usr/sbin/smbd";
const SSHD: &str = "/usr/sbin/sshd";
const TESTPARM: &str = "/usr/bin/testparm";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharingState {
    pub samba_installed: bool,
    pub ssh_installed: bool,
    pub avahi_installed: bool,
    pub file_sharing: bool,
    pub folders_allowed: bool,
    pub remote_login: bool,
    pub shares: Vec<Share>,
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharingCommand {
    Read,
    FileSharing(bool),
    RemoteLogin(bool),
    InstallSamba,
    InstallAvahi,
    AddFolder,
    Remove(String),
    Password(String),
}

fn service_active(unit: &str) -> bool {
    Command::new("/usr/bin/systemctl")
        .args(["is-active", "--quiet", unit])
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn parse_shares(listing: &str) -> Vec<Share> {
    let mut shares = Vec::new();
    let mut name = None;
    for line in listing.lines() {
        if let Some(found) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            name = Some(found.to_owned());
        } else if let (Some(path), Some(name)) = (line.strip_prefix("path="), name.take()) {
            shares.push(Share {
                name,
                path: PathBuf::from(path),
            });
        }
    }
    shares
}

fn read_shares() -> Vec<Share> {
    let listing = Command::new(PKEXEC)
        .args([HELPER, "list"])
        .stdin(Stdio::null())
        .output();
    match listing {
        Ok(output) if output.status.success() => {
            parse_shares(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(output) => {
            tracing::warn!(
                "cannot list the shared folders: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            Vec::new()
        }
        Err(error) => {
            tracing::warn!("cannot run the sharing helper: {error}");
            Vec::new()
        }
    }
}

fn folders_allowed() -> bool {
    let setting = Command::new(TESTPARM)
        .args(["-s", "--parameter-name", "usershare max shares"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match setting {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>()
            .is_ok_and(|most| most > 0),
        Ok(output) => {
            tracing::warn!("testparm cannot read Samba's setup: {}", output.status);
            false
        }
        Err(error) => {
            tracing::warn!("cannot run testparm: {error}");
            false
        }
    }
}

fn read_state() -> SharingState {
    let samba_installed = Path::new(SMBD).exists();
    let file_sharing = samba_installed && service_active("smbd.service");
    let folders_allowed = file_sharing && folders_allowed();
    SharingState {
        samba_installed,
        ssh_installed: Path::new(SSHD).exists(),
        avahi_installed: Path::new(AVAHI_DAEMON).exists(),
        file_sharing,
        folders_allowed,
        remote_login: service_active("ssh.service"),
        shares: match folders_allowed {
            true => read_shares(),
            false => Vec::new(),
        },
        failed: false,
    }
}

fn helper(arguments: &[&str], input: Option<&str>) -> bool {
    let child = Command::new(PKEXEC)
        .arg(HELPER)
        .args(arguments)
        .stdin(match input {
            Some(_) => Stdio::piped(),
            None => Stdio::null(),
        })
        .stdout(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!("cannot run the sharing helper: {error}");
            return false;
        }
    };
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take())
        && let Err(error) = writeln!(stdin, "{input}")
    {
        tracing::warn!("cannot pass the input to the sharing helper: {error}");
    }
    child.wait().is_ok_and(|status| status.success())
}

fn install(system: Option<&Connection>, package: &str) -> bool {
    system.is_some_and(|system| {
        Client::new(system)
            .install_missing(&[package])
            .inspect_err(|error| tracing::warn!("cannot install {package}: {error}"))
            .is_ok()
    })
}

fn share_name_for(path: &Path, taken: &[Share]) -> String {
    let base: String = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Shared".to_owned())
        .chars()
        .map(|c| match c.is_control() || "%<>*?|/\\+=;:\",".contains(c) {
            true => '-',
            false => c,
        })
        .collect();
    let mut name = base.clone();
    let mut number = 2;
    while taken
        .iter()
        .any(|share| share.name.eq_ignore_ascii_case(&name))
    {
        name = format!("{base} {number}");
        number += 1;
    }
    name
}

fn run(command: SharingCommand, system: Option<&Connection>, session: Option<&Connection>) -> bool {
    match command {
        SharingCommand::Read => true,
        SharingCommand::FileSharing(on) => helper(
            &[
                "file-sharing",
                match on {
                    true => "on",
                    false => "off",
                },
            ],
            None,
        ),
        SharingCommand::RemoteLogin(on) => helper(
            &[
                "remote-login",
                match on {
                    true => "on",
                    false => "off",
                },
            ],
            None,
        ),
        SharingCommand::InstallSamba => {
            install(system, SAMBA) && helper(&["samba-installed"], None)
        }
        SharingCommand::InstallAvahi => install(system, AVAHI),
        SharingCommand::Password(password) => helper(&["password"], Some(&password)),
        SharingCommand::Remove(name) => helper(&["remove", &name], None),
        SharingCommand::AddFolder => {
            let chosen = session.and_then(|session| {
                choose_file(session, &tr!("Choose a folder to share"), None)
                    .inspect_err(|error| tracing::warn!("cannot choose a folder: {error}"))
                    .ok()
                    .flatten()
            });
            let Some(path) = chosen else {
                return true;
            };
            let name = share_name_for(&path, &read_shares());
            helper(&["add", &name, &path.to_string_lossy()], None)
        }
    }
}

pub fn follow_sharing(
    session: Option<Connection>,
    system: Option<Connection>,
    updates: Updates,
) -> Sender<SharingCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("sharing", move || {
        for command in receiver {
            let worked = run(command, system.as_ref(), session.as_ref());
            let state = SharingState {
                failed: !worked,
                ..read_state()
            };
            if !updates.send(Update::Sharing(state)) {
                return;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn sharing_rows(&mut self) -> Widgets {
        let Some(state) = self.sharing.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        if state.failed {
            rows.push(crate::pages::muted_row(&tr!(
                "That change didn’t go through."
            )));
        }
        if state.ssh_installed {
            let mut change = intent(&self.intents, Intent::Sharing);
            let toggle = Toggle::new(&tr!("Remote Login"), state.remote_login, move |on| {
                change(SharingCommand::RemoteLogin(on));
            });
            let row = Row::new(
                Text::new(&tr!("Remote Login"), Tone::Primary),
                Some(boxed(toggle)),
            )
            .detail(
                Text::new(
                    &tr!("Others with an account on this computer can sign in with SSH"),
                    Tone::Muted,
                )
                .caption(),
            );
            rows.push(self.item_row("remote-login".to_owned(), row));
        }
        if !state.samba_installed {
            let mut install = intent(&self.intents, Intent::Sharing);
            let mut button = Button::new(&tr!("Install"), move || {
                install(SharingCommand::InstallSamba);
            });
            button.set_enabled(!self.installing_sharing);
            let detail = match self.installing_sharing {
                true => tr!("Installing…"),
                false => tr!("Samba, from the Debian archive"),
            };
            self.anchors
                .push((Anchor::Item("install-samba".to_owned()), button.id()));
            let row = Row::new(
                Text::new(&tr!("File Sharing"), Tone::Primary),
                Some(boxed(button)),
            )
            .detail(Text::new(&detail, Tone::Muted).caption());
            rows.push(boxed(row));
            return rows;
        }
        let mut change = intent(&self.intents, Intent::Sharing);
        let toggle = Toggle::new(&tr!("File Sharing"), state.file_sharing, move |on| {
            change(SharingCommand::FileSharing(on));
        });
        let row = Row::new(
            Text::new(&tr!("File Sharing"), Tone::Primary),
            Some(boxed(toggle)),
        )
        .detail(
            Text::new(
                &tr!("Other computers on this network can open the folders you share"),
                Tone::Muted,
            )
            .caption(),
        );
        rows.push(self.item_row("file-sharing".to_owned(), row));
        if !state.file_sharing {
            return rows;
        }
        if !state.folders_allowed {
            rows.push(crate::pages::muted_row(&tr!(
                "Samba’s setup on this computer doesn’t let people share their own folders. An administrator can allow it with “usershare max shares” in /etc/samba/smb.conf."
            )));
            return rows;
        }
        if !state.avahi_installed {
            rows.push(self.unadvertised_row());
        }
        let field = SecureField::new(
            &tr!("New password"),
            FIELD_SIZE,
            submit(&self.intents, |text| {
                Intent::Sharing(SharingCommand::Password(text))
            }),
        );
        self.anchors
            .push((Anchor::Item("sharing-password".to_owned()), field.id()));
        rows.push(boxed(
            Row::new(
                Text::new(&tr!("Sharing password"), Tone::Primary),
                Some(boxed(field)),
            )
            .detail(Text::new(&tr!("Others use your account name and this password to connect. Press Return to set it."), Tone::Muted).caption()),
        ));
        for share in &state.shares {
            let mut remove = intent(&self.intents, Intent::Sharing);
            let name = share.name.clone();
            let button: Box<dyn Widget> = boxed(Button::new(&tr!("Remove"), move || {
                remove(SharingCommand::Remove(name.clone()));
            }));
            let row = Row::new(Text::new(&share.name, Tone::Primary), Some(button))
                .detail(Text::new(&share.path.display().to_string(), Tone::Muted).caption());
            rows.push(self.item_row(format!("share:{}", share.name), row));
        }
        let mut add = intent(&self.intents, Intent::Sharing);
        let button = Button::new(&tr!("Add Folder…"), move || {
            add(SharingCommand::AddFolder);
        });
        rows.push(boxed(Row::new(
            Text::new(&tr!("Shared folders"), Tone::Primary),
            Some(boxed(button)),
        )));
        rows
    }

    fn unadvertised_row(&mut self) -> Box<dyn Widget> {
        let mut install = intent(&self.intents, Intent::Sharing);
        let mut button = Button::new(&tr!("Install"), move || {
            install(SharingCommand::InstallAvahi);
        });
        button.set_enabled(!self.installing_sharing);
        let detail = match self.installing_sharing {
            true => tr!("Installing…"),
            false => tr!(
                "Other computers won’t see this one nearby and must connect by its address. Installing Avahi lets it announce itself."
            ),
        };
        self.anchors
            .push((Anchor::Item("install-avahi".to_owned()), button.id()));
        boxed(
            Row::new(
                Text::new(&tr!("Not visible on the network"), Tone::Primary),
                Some(boxed(button)),
            )
            .detail(Text::new(&detail, Tone::Muted).caption()),
        )
    }
}
