use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use std::sync::mpsc::{self, Sender};

use bande_toolkit::ui::{Button, Label, SecureField, Widget};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use zeroize::Zeroizing;

use crate::L10N;
use crate::follow::{Command, Update, Updates, spawn};
use crate::pages::{FIELD_SIZE, Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const PASSWD: &str = "/usr/bin/passwd";
const STEP: Duration = Duration::from_secs(15);
const FIRST_UID: u32 = 1000;
const LAST_UID: u32 = 59999;
const ADMIN_GROUPS: [&str; 2] = ["sudo", "wheel"];
const NO_LOGIN: [&str; 2] = ["nologin", "false"];
const CURRENT_PROMPT: &str = "current password:";
const LEGACY_CURRENT_PROMPT: &str = "(current) unix password:";
const NEW_PROMPT: &str = "new password:";
const RETYPE_PROMPT: &str = "retype new password:";
const AUTHENTICATION: [&str; 2] = [
    "Authentication token manipulation error",
    "Authentication failure",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub login: String,
    pub name: String,
    pub admin: bool,
    pub you: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordOutcome {
    Changed,
    WrongPassword,
    Rejected(String),
    Failed(String),
}

pub(crate) fn members_of(groups: &[&str]) -> Vec<String> {
    std::fs::read_to_string("/etc/group")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let group = fields.next()?;
            let members = fields.nth(2)?;
            groups
                .contains(&group)
                .then(|| members.split(',').map(str::to_owned).collect::<Vec<_>>())
        })
        .flatten()
        .collect()
}

pub fn accounts() -> Vec<Account> {
    let uid = rustix::process::getuid().as_raw();
    let admins = members_of(&ADMIN_GROUPS);
    let mut accounts: Vec<Account> = std::fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            let (login, id, gecos, shell) = (
                fields.first()?,
                fields.get(2)?,
                fields.get(4)?,
                fields.get(6)?,
            );
            let id: u32 = id.parse().ok()?;
            let shell = shell.rsplit('/').next().unwrap_or_default();
            if !(FIRST_UID..=LAST_UID).contains(&id) || NO_LOGIN.contains(&shell) {
                return None;
            }
            let name = gecos.split(',').next().unwrap_or_default().trim();
            Some(Account {
                login: (*login).to_owned(),
                name: if name.is_empty() { *login } else { name }.to_owned(),
                admin: admins.iter().any(|admin| admin == login),
                you: id == uid,
            })
        })
        .collect();
    accounts.sort_by_key(|account| (!account.you, account.name.to_lowercase()));
    accounts
}

struct Session {
    master: File,
    child: Child,
    seen: String,
    closed: bool,
}

impl Session {
    fn start() -> io::Result<Self> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let path = ptsname(&master, Vec::new())?;
        let terminal = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.to_string_lossy().as_ref())?;
        let child = std::process::Command::new(PASSWD)
            .env("LC_ALL", "C")
            .stdin(Stdio::from(terminal.try_clone()?))
            .stdout(Stdio::from(terminal.try_clone()?))
            .stderr(Stdio::from(terminal))
            .spawn()?;
        Ok(Self {
            master: File::from(master),
            child,
            seen: String::new(),
            closed: false,
        })
    }

    fn wait_for(&mut self, prompts: &[&str]) -> Option<usize> {
        let until = Instant::now() + STEP;
        loop {
            let tail = self.seen.trim_end().to_lowercase();
            if let Some(found) = prompts.iter().position(|prompt| tail.ends_with(prompt)) {
                return Some(found);
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let timeout = Timespec {
                tv_sec: i64::try_from(left.as_secs()).unwrap_or(0),
                tv_nsec: i64::from(left.subsec_nanos()),
            };
            let mut fds = [PollFd::new(&self.master, PollFlags::IN)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) => return None,
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(_) => return None,
            }
            let mut buffer = [0u8; 512];
            match self.master.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    self.closed = true;
                    return None;
                }
                Ok(read) => self
                    .seen
                    .push_str(&String::from_utf8_lossy(&buffer[..read])),
            }
        }
    }

    fn answer(&mut self, text: &str) -> io::Result<()> {
        self.seen.clear();
        self.master.write_all(text.as_bytes())?;
        self.master.write_all(b"\n")
    }

    fn message(&self) -> String {
        self.seen
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.to_lowercase().ends_with("password:"))
            .map(|line| {
                line.trim_start_matches("passwd: ")
                    .trim_start_matches("BAD PASSWORD: ")
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn finish(&mut self) -> io::Result<bool> {
        self.wait_for(&[]);
        Ok(self.closed && self.child.wait()?.success())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn change_password(current: &str, new: &str) -> PasswordOutcome {
    let failed = |error: io::Error| PasswordOutcome::Failed(error.to_string());
    let mut session = match Session::start() {
        Ok(session) => session,
        Err(error) => return failed(error),
    };
    if matches!(
        session.wait_for(&[CURRENT_PROMPT, LEGACY_CURRENT_PROMPT, NEW_PROMPT]),
        Some(0 | 1)
    ) {
        if let Err(error) = session.answer(current) {
            return failed(error);
        }
        if session.wait_for(&[NEW_PROMPT]).is_none() {
            let message = session.message();
            return if AUTHENTICATION
                .iter()
                .any(|failure| message.contains(failure))
            {
                PasswordOutcome::WrongPassword
            } else {
                PasswordOutcome::Failed(message)
            };
        }
    }
    if let Err(error) = session.answer(new) {
        return failed(error);
    }
    if session.wait_for(&[RETYPE_PROMPT, NEW_PROMPT]) != Some(0) {
        return PasswordOutcome::Rejected(session.message());
    }
    if let Err(error) = session.answer(new) {
        return failed(error);
    }
    match session.finish() {
        Ok(true) => PasswordOutcome::Changed,
        Ok(false) => PasswordOutcome::Failed(session.message()),
        Err(error) => failed(error),
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum UsersCommand {
    Refresh,
    ChangePassword(Zeroizing<String>, Zeroizing<String>),
}

impl std::fmt::Debug for UsersCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Refresh => "Refresh",
            Self::ChangePassword(..) => "ChangePassword",
        })
    }
}

pub fn follow_users(updates: Updates) -> Sender<UsersCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("users", move || {
        for command in receiver {
            let update = match command {
                UsersCommand::Refresh => Update::Accounts(accounts()),
                UsersCommand::ChangePassword(current, new) => {
                    Update::Password(change_password(&current, &new))
                }
            };
            if !updates.send(update) {
                break;
            }
        }
    });
    sender
}

#[derive(Clone, PartialEq, Eq, Default)]
pub(crate) struct PasswordForm {
    pub current: Option<Zeroizing<String>>,
    pub new: Option<Zeroizing<String>>,
    pub busy: bool,
    pub error: Option<String>,
}

const PASSWORD_FIELDS: [&str; 3] = ["password-current", "password-new", "password-verify"];

impl SettingsView {
    pub(crate) fn account_rows(&mut self) -> Widgets {
        let Some(accounts) = self.accounts.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for account in accounts {
            let kind = L10N.tr(if account.admin {
                "account-admin"
            } else {
                "account-standard"
            });
            let detail = format!("{} · {kind}", account.login);
            let control = if account.you && self.password.is_none() {
                let mut ask = intent(&self.intents, |()| Intent::AskPassword(true));
                let button = Button::new(&L10N.tr("change-password-ellipsis"), move || ask(()));
                self.anchors
                    .push((Anchor::Item("change-password".to_owned()), button.id()));
                Some(boxed(button))
            } else if account.you {
                None
            } else {
                Some(boxed(Label::new(&L10N.tr("account-other"))))
            };
            let title = if account.you {
                L10N.tr_args("account-you", [("name", account.name.clone().into())])
            } else {
                account.name.clone()
            };
            rows.push(boxed(
                Row::new(Text::new(&title, Tone::Primary), control)
                    .detail(Text::new(&detail, Tone::Muted).caption()),
            ));
            if account.you && self.password.is_some() {
                rows.extend(self.password_rows());
            }
        }
        if let Some(message) = &self.password_message {
            rows.push(muted_row(message));
        }
        rows
    }

    fn password_rows(&mut self) -> Widgets {
        let Some(form) = self.password.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for (step, anchor) in PASSWORD_FIELDS.iter().enumerate() {
            let label = L10N.tr(anchor);
            let mut entry = intent(&self.intents, move |text: String| {
                Intent::PasswordEntry(step, text)
            });
            let field =
                SecureField::new(&label, FIELD_SIZE, move |text: &str| entry(text.to_owned()));
            self.anchors
                .push((Anchor::Item((*anchor).to_owned()), field.id()));
            rows.push(boxed(Row::new(
                Text::new(&label, Tone::Primary),
                Some(boxed(field)),
            )));
        }
        let detail = match (&form.error, form.busy) {
            (_, true) => L10N.tr("password-changing"),
            (Some(error), false) => error.clone(),
            (None, false) => L10N.tr("password-steps"),
        };
        let mut cancel = intent(&self.intents, |()| Intent::AskPassword(false));
        rows.push(boxed(Row::new(
            Text::new(&detail, Tone::Muted).caption(),
            Some(boxed(
                Button::new(&L10N.tr("cancel"), move || cancel(())).cancel(),
            )),
        )));
        rows
    }

    pub(crate) fn ask_password(&mut self, asking: bool) {
        self.password_message = None;
        self.password = asking.then(PasswordForm::default);
        self.focus_next = Some(Anchor::Item(
            if asking {
                PASSWORD_FIELDS[0]
            } else {
                "change-password"
            }
            .to_owned(),
        ));
    }

    pub(crate) fn password_entry(&mut self, step: usize, text: String) {
        let Some(form) = self.password.as_mut().filter(|form| !form.busy) else {
            return;
        };
        let text = Zeroizing::new(text);
        let next = match step {
            0 => {
                form.current = Some(text);
                Some(1)
            }
            1 if text.is_empty() => {
                form.error = Some(L10N.tr("password-empty"));
                Some(1)
            }
            1 => {
                form.new = Some(text);
                Some(2)
            }
            _ => match (&form.current, &form.new) {
                (Some(current), Some(new)) if *new == text => {
                    form.busy = true;
                    form.error = None;
                    self.commands
                        .push(Command::Users(UsersCommand::ChangePassword(
                            current.clone(),
                            new.clone(),
                        )));
                    None
                }
                (None, _) => {
                    form.error = Some(L10N.tr("password-current-needed"));
                    Some(0)
                }
                _ => {
                    form.new = None;
                    form.error = Some(L10N.tr("password-mismatch"));
                    Some(1)
                }
            },
        };
        if let Some(next) = next.and_then(|next| PASSWORD_FIELDS.get(next)) {
            self.focus_next = Some(Anchor::Item((*next).to_owned()));
        }
    }

    pub(crate) fn password_changed(&mut self, outcome: PasswordOutcome) {
        let Some(form) = self.password.as_mut() else {
            return;
        };
        form.busy = false;
        let (error, field) = match outcome {
            PasswordOutcome::Changed => {
                self.password = None;
                self.password_message = Some(L10N.tr("password-changed"));
                self.focus_next = Some(Anchor::Item("change-password".to_owned()));
                return;
            }
            PasswordOutcome::WrongPassword => {
                form.current = None;
                (L10N.tr("password-wrong"), 0)
            }
            PasswordOutcome::Rejected(reason) if !reason.is_empty() => {
                form.new = None;
                (
                    L10N.tr_args("password-rejected", [("reason", reason.into())]),
                    1,
                )
            }
            PasswordOutcome::Rejected(_) => {
                form.new = None;
                (L10N.tr("password-weak"), 1)
            }
            PasswordOutcome::Failed(reason) => {
                tracing::warn!("passwd: {reason}");
                (L10N.tr("couldnt-change"), 0)
            }
        };
        form.error = Some(error);
        self.focus_next = Some(Anchor::Item(PASSWORD_FIELDS[field].to_owned()));
    }
}
