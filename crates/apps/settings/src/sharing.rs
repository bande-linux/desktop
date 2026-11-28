use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc::{self, Sender};

use bande_app::choose_folder;
use bande_keyring::Keyring;
use bande_l10n::tr;
use bande_packages::Client;
use bande_screen_sharing::setup::{
    self as screen_sharing, KEYRING_APPLICATION, PASSWORD_KEY, SHORTEST_PASSWORD,
};
use bande_settings_client::ConfigProxyBlocking;
use bande_toolkit::Spacing;
use bande_toolkit::ui::{
    Alert, Axis, Button, CheckState, Checkbox, SecureField, Stack, Toggle, Widget,
};
use zbus::blocking::Connection;

use crate::follow::{Command as Order, Update, Updates, spawn};
use crate::pages::{FIELD_SIZE, Widgets, boxed, submit};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const HELPER: &str = "/usr/libexec/bande-sharing-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const SAMBA: &str = "samba";
const SCREEN_SHARING: &str = "bande-screen-sharing";
const SYSTEMCTL: &str = "/usr/bin/systemctl";
const AVAHI: &str = "avahi-daemon";
const AVAHI_DAEMON: &str = "/usr/sbin/avahi-daemon";
const SMBD: &str = "/usr/sbin/smbd";
const SSHD: &str = "/usr/sbin/sshd";
const TESTPARM: &str = "/usr/bin/testparm";
const TURN_ON: &str = "turn-on";
const CANCEL: &str = "cancel";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Share {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenSharing {
    pub on: bool,
    pub running: bool,
    pub password_set: bool,
    pub ask: bool,
    pub older_viewers: bool,
    pub fingerprint: Option<String>,
    pub addresses: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Failed,
    ShortPassword,
    NoPassword,
    Keyring,
    NotStarted,
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
    pub screen: Option<ScreenSharing>,
    pub problem: Option<Problem>,
    pub screen_problem: Option<Problem>,
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
    InstallScreenSharing,
    ScreenSharing(bool),
    TurnOnWithPassword(String),
    ScreenPassword(String),
    AskBeforeViewing(bool),
    OlderViewers(bool),
}

fn systemctl(arguments: &[&str]) -> bool {
    Command::new(SYSTEMCTL)
        .args(arguments)
        .stdin(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn service_active(unit: &str) -> bool {
    systemctl(&["is-active", "--quiet", unit])
}

fn screen_password() -> Option<String> {
    Keyring::open(KEYRING_APPLICATION)
        .and_then(|keyring| keyring.password(&PASSWORD_KEY))
        .inspect_err(|error| tracing::warn!("cannot read the Screen Sharing password: {error}"))
        .ok()
        .flatten()
        .filter(|password| !password.is_empty())
}

fn read_screen_sharing(session: Option<&Connection>) -> Option<ScreenSharing> {
    if !Path::new(screen_sharing::PROGRAM).exists() {
        return None;
    }
    let config = session.and_then(|session| {
        ConfigProxyBlocking::new(session)
            .inspect_err(|error| tracing::warn!("cannot reach the settings service: {error}"))
            .ok()
    });
    let fingerprint = screen_sharing::certificate_file()
        .and_then(|file| std::fs::read(file).ok())
        .map(|certificate| screen_sharing::fingerprint(&certificate));
    Some(ScreenSharing {
        on: systemctl(&["--user", "is-enabled", "--quiet", screen_sharing::UNIT]),
        running: systemctl(&["--user", "is-active", "--quiet", screen_sharing::UNIT]),
        password_set: screen_password().is_some(),
        ask: config
            .as_ref()
            .and_then(|config| config.screen_sharing_ask().ok())
            .unwrap_or(true),
        older_viewers: config
            .as_ref()
            .and_then(|config| config.screen_sharing_older_viewers().ok())
            .unwrap_or(false),
        fingerprint,
        addresses: bande_screen_sharing::network::addresses()
            .iter()
            .map(ToString::to_string)
            .collect(),
    })
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

fn read_state(session: Option<&Connection>) -> SharingState {
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
        screen: read_screen_sharing(session),
        problem: None,
        screen_problem: None,
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

fn turn_screen_sharing(on: bool) -> Result<(), Problem> {
    if !on {
        return worked(systemctl(&[
            "--user",
            "disable",
            "--now",
            screen_sharing::UNIT,
        ]));
    }
    if screen_password().is_none() {
        return Err(Problem::NoPassword);
    }
    let started = systemctl(&["--user", "enable", "--now", screen_sharing::UNIT])
        && systemctl(&["--user", "is-active", "--quiet", screen_sharing::UNIT]);
    if started {
        return Ok(());
    }
    if !systemctl(&["--user", "disable", "--now", screen_sharing::UNIT]) {
        tracing::warn!("cannot turn Screen Sharing off again after it failed to start");
    }
    Err(Problem::NotStarted)
}

fn save_screen_password(password: &str) -> Result<(), Problem> {
    if password.chars().count() < SHORTEST_PASSWORD {
        return Err(Problem::ShortPassword);
    }
    Keyring::open(KEYRING_APPLICATION)
        .and_then(|keyring| keyring.save(&PASSWORD_KEY, &tr!("Screen Sharing password"), password))
        .map_err(|error| {
            tracing::warn!("cannot keep the Screen Sharing password: {error}");
            Problem::Keyring
        })?;
    worked(systemctl(&["--user", "try-restart", screen_sharing::UNIT]))
}

fn configure(
    session: Option<&Connection>,
    change: impl FnOnce(&ConfigProxyBlocking<'_>) -> zbus::Result<()>,
) -> Result<(), Problem> {
    session
        .ok_or(Problem::Failed)
        .and_then(|session| ConfigProxyBlocking::new(session).map_err(|_| Problem::Failed))
        .and_then(|config| {
            change(&config).map_err(|error| {
                tracing::warn!("cannot change the Screen Sharing settings: {error}");
                Problem::Failed
            })
        })
}

fn worked(done: bool) -> Result<(), Problem> {
    match done {
        true => Ok(()),
        false => Err(Problem::Failed),
    }
}

fn run(
    command: SharingCommand,
    system: Option<&Connection>,
    session: Option<&Connection>,
) -> Result<(), Problem> {
    match command {
        SharingCommand::InstallScreenSharing => worked(install(system, SCREEN_SHARING)),
        SharingCommand::ScreenSharing(on) => turn_screen_sharing(on),
        SharingCommand::TurnOnWithPassword(password) => {
            save_screen_password(&password)?;
            turn_screen_sharing(true)
        }
        SharingCommand::ScreenPassword(password) => save_screen_password(&password),
        SharingCommand::AskBeforeViewing(ask) => {
            configure(session, |config| config.set_screen_sharing_ask(ask))
        }
        SharingCommand::OlderViewers(allowed) => configure(session, |config| {
            config.set_screen_sharing_older_viewers(allowed)
        }),
        command => worked(run_file_sharing(command, system, session)),
    }
}

fn run_file_sharing(
    command: SharingCommand,
    system: Option<&Connection>,
    session: Option<&Connection>,
) -> bool {
    match command {
        SharingCommand::Read
        | SharingCommand::InstallScreenSharing
        | SharingCommand::ScreenSharing(_)
        | SharingCommand::TurnOnWithPassword(_)
        | SharingCommand::ScreenPassword(_)
        | SharingCommand::AskBeforeViewing(_)
        | SharingCommand::OlderViewers(_) => true,
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
            if !helper(&["samba-installing"], None) {
                return false;
            }
            if install(system, SAMBA) {
                return helper(&["samba-installed"], None);
            }
            if !helper(&["samba-not-installed"], None) {
                tracing::warn!("smbd and nmbd stay masked after Samba was not installed");
            }
            false
        }
        SharingCommand::InstallAvahi => install(system, AVAHI),
        SharingCommand::Password(password) => helper(&["password"], Some(&password)),
        SharingCommand::Remove(name) => helper(&["remove", &name], None),
        SharingCommand::AddFolder => {
            let chosen = session.and_then(|session| {
                choose_folder(session, &tr!("Choose a folder to share"))
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
        let mut screen_problem = None;
        for command in receiver {
            let turning = matches!(
                command,
                SharingCommand::ScreenSharing(_)
                    | SharingCommand::TurnOnWithPassword(_)
                    | SharingCommand::ScreenPassword(_)
            );
            let outcome = run(command, system.as_ref(), session.as_ref());
            let problem = match turning {
                true => {
                    screen_problem = outcome.err();
                    None
                }
                false => outcome.err(),
            };
            let state = SharingState {
                problem,
                screen_problem,
                ..read_state(session.as_ref())
            };
            if !updates.send(Update::Sharing(state)) {
                return;
            }
        }
    });
    sender
}

fn problem_text(problem: Problem) -> String {
    match problem {
        Problem::Failed => tr!("That change didn’t go through."),
        Problem::ShortPassword => tr!(
            "Use at least {count} characters.",
            count = SHORTEST_PASSWORD
        ),
        Problem::NoPassword => tr!("Choose a password first."),
        Problem::Keyring => {
            tr!("The keyring is locked or unavailable, so the password can’t be kept.")
        }
        Problem::NotStarted => {
            tr!("Screen Sharing didn’t start. Details: journalctl --user -u bande-screen-sharing")
        }
    }
}

impl SettingsView {
    pub(crate) fn sharing_rows(&mut self) -> Widgets {
        let Some(state) = self.sharing.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        if let Some(problem) = state.problem {
            rows.push(crate::pages::muted_row(&problem_text(problem)));
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
        rows.extend(self.screen_sharing_rows(&state));
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

    fn screen_sharing_rows(&mut self, state: &SharingState) -> Widgets {
        let Some(screen) = &state.screen else {
            let mut install = intent(&self.intents, Intent::Sharing);
            let mut button = Button::new(&tr!("Install"), move || {
                install(SharingCommand::InstallScreenSharing);
            });
            button.set_enabled(!self.installing_sharing);
            let detail = match self.installing_sharing {
                true => tr!("Installing…"),
                false => tr!("Lets others on the local network see and control this screen"),
            };
            self.anchors.push((
                Anchor::Item("install-screen-sharing".to_owned()),
                button.id(),
            ));
            return vec![boxed(
                Row::new(
                    Text::new(&tr!("Screen Sharing"), Tone::Primary),
                    Some(boxed(button)),
                )
                .detail(Text::new(&detail, Tone::Muted).caption()),
            )];
        };
        let mut rows = Vec::new();
        let mut change = intent(&self.intents, Intent::Sharing);
        let on = screen.on || self.turning_on_screen;
        let toggle = Toggle::new(&tr!("Screen Sharing"), on, move |on| {
            change(SharingCommand::ScreenSharing(on));
        });
        let detail = match (self.turning_on_screen, screen.on, screen.addresses.first()) {
            (true, _, _) => Text::new(&tr!("Turning on…"), Tone::Muted),
            (false, _, _) if state.screen_problem.is_some() => Text::new(
                &state.screen_problem.map(problem_text).unwrap_or_default(),
                Tone::Danger,
            ),
            (false, true, Some(address)) => Text::new(
                &tr!(
                    "VNC viewers on the local network connect to vnc://{address}",
                    address = address.clone()
                ),
                Tone::Muted,
            ),
            _ => Text::new(
                &tr!(
                    "Others on the local network can see and control this screen with a VNC viewer"
                ),
                Tone::Muted,
            ),
        };
        let row = Row::new(
            Text::new(&tr!("Screen Sharing"), Tone::Primary),
            Some(boxed(toggle)),
        )
        .detail(detail.caption());
        rows.push(self.item_row("screen-sharing".to_owned(), row));
        if screen.on && !screen.running {
            rows.push(crate::pages::muted_row(&tr!(
                "Screen Sharing couldn’t start. Another program may be using port 5900."
            )));
        }
        let field = SecureField::new(
            &tr!("New password"),
            FIELD_SIZE,
            submit(&self.intents, |text| {
                Intent::Sharing(SharingCommand::ScreenPassword(text))
            }),
        );
        self.anchors.push((
            Anchor::Item("screen-sharing-password".to_owned()),
            field.id(),
        ));
        let detail = match screen.password_set {
            true => tr!("Viewers type this password. Press Return to change it."),
            false => tr!(
                "Choose a password of at least {count} characters and press Return.",
                count = SHORTEST_PASSWORD
            ),
        };
        rows.push(boxed(
            Row::new(
                Text::new(&tr!("Screen Sharing password"), Tone::Primary),
                Some(boxed(field)),
            )
            .detail(Text::new(&detail, Tone::Muted).caption()),
        ));
        if !screen.on {
            return rows;
        }
        let mut change = intent(&self.intents, Intent::Sharing);
        let toggle = Toggle::new(
            &tr!("Ask before someone connects"),
            screen.ask,
            move |ask| {
                change(SharingCommand::AskBeforeViewing(ask));
            },
        );
        let row = Row::new(
            Text::new(&tr!("Ask before someone connects"), Tone::Primary),
            Some(boxed(toggle)),
        )
        .detail(
            Text::new(
                &tr!("A viewer that knows the password waits until you allow it"),
                Tone::Muted,
            )
            .caption(),
        );
        rows.push(self.item_row("screen-sharing-ask".to_owned(), row));
        let mut change = intent(&self.intents, Intent::Sharing);
        let toggle = Toggle::new(
            &tr!("Allow older viewers"),
            screen.older_viewers,
            move |allowed| change(SharingCommand::OlderViewers(allowed)),
        );
        let row = Row::new(
            Text::new(&tr!("Allow older viewers"), Tone::Primary),
            Some(boxed(toggle)),
        )
        .detail(
            Text::new(
                &tr!(
                    "Viewers without encryption check only the first 8 characters of the password, and what is on the screen travels unencrypted. Use it only on a network you trust."
                ),
                Tone::Muted,
            )
            .caption(),
        );
        rows.push(self.item_row("screen-sharing-older".to_owned(), row));
        if let Some(fingerprint) = &screen.fingerprint {
            rows.push(boxed(
                Row::new(
                    Text::new(&tr!("Certificate fingerprint"), Tone::Primary),
                    None,
                )
                .detail(
                    Text::new(
                        &tr!(
                            "A viewer shows it the first time it connects: {fingerprint}",
                            fingerprint = fingerprint.clone()
                        ),
                        Tone::Muted,
                    )
                    .caption(),
                ),
            ));
        }
        if !state.avahi_installed {
            rows.push(self.unadvertised_row());
        }
        rows
    }

    pub(crate) fn needs_screen_password(&self) -> bool {
        self.sharing
            .as_ref()
            .and_then(|state| state.screen.as_ref())
            .is_some_and(|screen| !screen.password_set)
    }

    pub(crate) fn ask_screen_password(&mut self, problem: Option<String>) {
        self.screen_password.borrow_mut().clear();
        let typed = Rc::clone(&self.screen_password);
        let mut submitted = intent(&self.intents, Intent::ScreenPasswordAnswer);
        let field = SecureField::new(&tr!("Password"), FIELD_SIZE, move |_| {
            submitted(TURN_ON.to_owned());
        })
        .on_edit(move |text| {
            let mut typed = typed.borrow_mut();
            typed.clear();
            typed.push_str(text);
        });
        self.screen_password_field = Some(field.id());
        let mut reveal = intent(&self.intents, Intent::RevealScreenPassword);
        let show = Checkbox::new(&tr!("Show password"), CheckState::Off, move |shown| {
            reveal(shown);
        });
        let message = problem.unwrap_or_else(|| {
            tr!(
                "Viewers type it to connect. At least {count} characters.",
                count = SHORTEST_PASSWORD
            )
        });
        let mut answer = intent(&self.intents, Intent::ScreenPasswordAnswer);
        let alert = Alert::new(
            &tr!("Choose a password for Screen Sharing"),
            &message,
            move |choice| answer(choice.to_owned()),
        )
        .accessory(Stack::new(
            Axis::Vertical,
            Spacing::S as f32,
            0.0,
            vec![boxed(field), boxed(show)],
        ))
        .primary(&tr!("Turn On"), TURN_ON)
        .cancel(&tr!("Cancel"), CANCEL);
        self.ui.present_sheet(alert);
        if let Some(field) = self.screen_password_field {
            self.ui.set_focus(field);
        }
        self.invalidate();
    }

    pub(crate) fn reveal_screen_password(&mut self, shown: bool) {
        if let Some(field) = self
            .screen_password_field
            .and_then(|field| self.ui.widget_as::<SecureField>(field))
        {
            field.set_revealed(shown);
        }
    }

    pub(crate) fn screen_password_answered(&mut self, choice: &str) {
        self.ui.close_sheet();
        self.screen_password_field = None;
        let password = std::mem::take(&mut *self.screen_password.borrow_mut());
        if choice != TURN_ON {
            self.invalidate();
            return;
        }
        if password.chars().count() < SHORTEST_PASSWORD {
            self.ask_screen_password(Some(tr!(
                "Use at least {count} characters.",
                count = SHORTEST_PASSWORD
            )));
            return;
        }
        self.turning_on_screen = true;
        self.commands
            .push(Order::Sharing(SharingCommand::TurnOnWithPassword(
                password.to_string(),
            )));
        self.invalidate();
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
