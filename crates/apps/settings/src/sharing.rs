use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};

use bande_app::choose_file;
use bande_packages::{Client, Info, Mode};
use bande_toolkit::ui::{Button, SecureField, Toggle, Widget};
use zbus::blocking::Connection;

use crate::L10N;
use crate::follow::{Update, Updates, spawn};
use crate::pages::{FIELD_SIZE, Widgets, boxed, submit};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const HELPER: &str = "/usr/libexec/bande-sharing-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const SAMBA: &str = "samba";
const SMBD: &str = "/usr/sbin/smbd";
const SSHD: &str = "/usr/sbin/sshd";
const NET: &str = "/usr/bin/net";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharingState {
    pub samba_installed: bool,
    pub ssh_installed: bool,
    pub file_sharing: bool,
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
    Command::new(NET)
        .args(["usershare", "info"])
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|output| parse_shares(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
}

fn read_state() -> SharingState {
    let samba_installed = Path::new(SMBD).exists();
    SharingState {
        samba_installed,
        ssh_installed: Path::new(SSHD).exists(),
        file_sharing: samba_installed && service_active("smbd.service"),
        remote_login: service_active("ssh.service"),
        shares: match samba_installed {
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
    if let (Some(input), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = writeln!(stdin, "{input}");
    }
    child.wait().is_ok_and(|status| status.success())
}

fn install_samba(system: &Connection) -> Result<(), bande_packages::Error> {
    let client = Client::new(system);
    let ids: Vec<String> = client
        .resolve(&[SAMBA.to_owned()])?
        .into_iter()
        .filter(|package| package.info == Info::Available)
        .map(|package| package.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    client.install(&ids, Mode::Apply, &|_| {}).map(drop)
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
        SharingCommand::InstallSamba => system.is_some_and(|system| {
            install_samba(system)
                .inspect_err(|error| tracing::warn!("cannot install Samba: {error}"))
                .is_ok()
        }),
        SharingCommand::Password(password) => helper(&["password"], Some(&password)),
        SharingCommand::Remove(name) => helper(&["remove", &name], None),
        SharingCommand::AddFolder => {
            let chosen = session.and_then(|session| {
                choose_file(session, &L10N.tr("sharing-choose-folder"), None)
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
            rows.push(crate::pages::muted_row(&L10N.tr("sharing-failed")));
        }
        if state.ssh_installed {
            let mut change = intent(&self.intents, Intent::Sharing);
            let toggle = Toggle::new(
                &L10N.tr("sharing-remote-login"),
                state.remote_login,
                move |on| {
                    change(SharingCommand::RemoteLogin(on));
                },
            );
            let row = Row::new(
                Text::new(&L10N.tr("sharing-remote-login"), Tone::Primary),
                Some(boxed(toggle)),
            )
            .detail(Text::new(&L10N.tr("sharing-remote-login-detail"), Tone::Muted).caption());
            rows.push(self.item_row("remote-login".to_owned(), row));
        }
        if !state.samba_installed {
            let mut install = intent(&self.intents, Intent::Sharing);
            let mut button = Button::new(&L10N.tr("sharing-install"), move || {
                install(SharingCommand::InstallSamba);
            });
            button.set_enabled(!self.installing_samba);
            let detail = match self.installing_samba {
                true => L10N.tr("sharing-installing"),
                false => L10N.tr("sharing-install-detail"),
            };
            self.anchors
                .push((Anchor::Item("install-samba".to_owned()), button.id()));
            let row = Row::new(
                Text::new(&L10N.tr("sharing-file-sharing"), Tone::Primary),
                Some(boxed(button)),
            )
            .detail(Text::new(&detail, Tone::Muted).caption());
            rows.push(boxed(row));
            return rows;
        }
        let mut change = intent(&self.intents, Intent::Sharing);
        let toggle = Toggle::new(
            &L10N.tr("sharing-file-sharing"),
            state.file_sharing,
            move |on| {
                change(SharingCommand::FileSharing(on));
            },
        );
        let row = Row::new(
            Text::new(&L10N.tr("sharing-file-sharing"), Tone::Primary),
            Some(boxed(toggle)),
        )
        .detail(Text::new(&L10N.tr("sharing-file-sharing-detail"), Tone::Muted).caption());
        rows.push(self.item_row("file-sharing".to_owned(), row));
        if !state.file_sharing {
            return rows;
        }
        let field = SecureField::new(
            &L10N.tr("sharing-password"),
            FIELD_SIZE,
            submit(&self.intents, |text| {
                Intent::Sharing(SharingCommand::Password(text))
            }),
        );
        self.anchors
            .push((Anchor::Item("sharing-password".to_owned()), field.id()));
        rows.push(boxed(
            Row::new(
                Text::new(&L10N.tr("sharing-password-label"), Tone::Primary),
                Some(boxed(field)),
            )
            .detail(Text::new(&L10N.tr("sharing-password-detail"), Tone::Muted).caption()),
        ));
        for share in &state.shares {
            let mut remove = intent(&self.intents, Intent::Sharing);
            let name = share.name.clone();
            let button: Box<dyn Widget> =
                boxed(Button::new(&L10N.tr("sharing-remove"), move || {
                    remove(SharingCommand::Remove(name.clone()));
                }));
            let row = Row::new(Text::new(&share.name, Tone::Primary), Some(button))
                .detail(Text::new(&share.path.display().to_string(), Tone::Muted).caption());
            rows.push(self.item_row(format!("share:{}", share.name), row));
        }
        let mut add = intent(&self.intents, Intent::Sharing);
        let button = Button::new(&L10N.tr("sharing-add"), move || {
            add(SharingCommand::AddFolder);
        });
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("sharing-folders"), Tone::Primary),
            Some(boxed(button)),
        )));
        rows
    }
}
