use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_app::choose_file;
use bande_backup::client::{
    BackupProxy, DiskChoice, Status, follow_status, phase, problem, setup_error,
};
use bande_datetime::{DateFormats, local_zone, now};
use bande_fs::format_size;
use bande_toolkit::Spacing;
use bande_toolkit::ui::{
    Axis, Button, ProgressBar, SecureField, Spinner, Stack, TextField, Toggle, Widget,
};
use bande_volumes::udisks::Udisks;
use zbus::blocking::Connection;
use zeroize::Zeroizing;

use crate::L10N;
use crate::follow::{Command, Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::passwords::Concealed;
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const FIELD_SIZE: f32 = 13.0;
const PASSWORD_ANCHOR: &str = "backup-password";
const SECONDS_PER_MINUTE: u64 = 60;
const NETWORK_ANCHOR: &str = "backup-network";
const NEW_PLACE: &str = "new";
const EXISTING_PLACE: &str = "existing";

#[derive(Debug, Clone, PartialEq)]
pub enum BackupCommand {
    Open,
    Inspect(DiskChoice),
    Choose(String, Concealed),
    Erase(String, Concealed),
    CheckNetwork(String, String, Concealed),
    ChooseNetwork(String, String, Concealed, Concealed),
    AddExcluded(Vec<String>),
    SetExcluded(Vec<String>),
    StopUsing,
    BackUpNow,
    Pause,
    Resume,
    OnBattery(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackupUpdate {
    Status(Status),
    Absent,
    Disks(Vec<DiskChoice>),
    Inspected(DiskChoice, bool),
    NetworkChecked(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BackupIntent {
    Pick(DiskChoice),
    Password(Concealed),
    Cancel,
    Command(BackupCommand),
    AskStop(bool),
    AskErase(DiskChoice),
    EraseConfirmed,
    StartNetwork,
    NetworkDraft(String),
    NetworkNext,
    NetworkPassword(Concealed),
}

#[derive(Debug, Clone)]
enum Target {
    Disk(DiskChoice),
    Erase(DiskChoice),
    Network {
        url: String,
        user: String,
        share_password: Concealed,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkStage {
    Address,
    User,
    Password,
}

#[derive(Debug, Clone)]
struct NetworkDraft {
    stage: NetworkStage,
    url: String,
    user: String,
    text: String,
    share_password: Option<Concealed>,
    error: Option<&'static str>,
}

#[derive(Debug, Default)]
pub struct BackupView {
    status: Option<Status>,
    absent: bool,
    disks: Vec<DiskChoice>,
    choosing: Option<(Target, bool)>,
    asking_erase: Option<DiskChoice>,
    network: Option<NetworkDraft>,
    first_password: Option<Concealed>,
    passwords_differ: bool,
    confirming_stop: bool,
}

fn run_one(
    proxy: &BackupProxy<'_>,
    session: &Connection,
    updates: &Updates,
    command: BackupCommand,
) -> zbus::Result<()> {
    match command {
        BackupCommand::Open => send_disks(proxy, updates),
        BackupCommand::Inspect(disk) => {
            let found = proxy.has_backups(&disk.uuid)?;
            updates.send(Update::Backup(BackupUpdate::Inspected(disk, found)));
            Ok(())
        }
        BackupCommand::Choose(uuid, password) => proxy.choose(&uuid, &password.0),
        BackupCommand::Erase(block, password) => proxy.erase_and_choose(&block, &password.0),
        BackupCommand::CheckNetwork(url, user, secret) => {
            let answer = proxy.check_network(&url, &user, &secret.0)?;
            updates.send(Update::Backup(BackupUpdate::NetworkChecked(answer)));
            Ok(())
        }
        BackupCommand::ChooseNetwork(url, user, secret, password) => {
            proxy.choose_network(&url, &user, &secret.0, &password.0)
        }
        BackupCommand::SetExcluded(folders) => proxy.set_excluded(folders),
        BackupCommand::AddExcluded(current) => {
            let Some(folder) = choose_file(session, &L10N.tr("choose-excluded-folder"), None)?
            else {
                return Ok(());
            };
            let mut folders = current;
            let folder = folder.to_string_lossy().into_owned();
            if !folders.contains(&folder) {
                folders.push(folder);
            }
            proxy.set_excluded(folders)
        }
        BackupCommand::StopUsing => proxy.stop_using(),
        BackupCommand::BackUpNow => proxy.back_up_now(),
        BackupCommand::Pause => proxy.pause(),
        BackupCommand::Resume => proxy.resume(),
        BackupCommand::OnBattery(allowed) => proxy.set_on_battery(allowed),
    }
}

fn send_disks(proxy: &BackupProxy<'_>, updates: &Updates) -> zbus::Result<()> {
    let disks = proxy.disks()?;
    updates.send(Update::Backup(BackupUpdate::Disks(disks)));
    Ok(())
}

fn start_following(system: &Connection, updates: &Updates, commands: Sender<BackupCommand>) {
    let statuses = updates.clone();
    let followed = follow_status(system, move |status| match status {
        Ok(status) => statuses.send(Update::Backup(BackupUpdate::Status(status))),
        Err(error) => {
            tracing::debug!("cannot read the backup status: {error}");
            statuses.send(Update::Backup(BackupUpdate::Absent))
        }
    });
    if let Err(error) = followed {
        tracing::warn!("cannot follow the backup service: {error}");
        updates.send(Update::Backup(BackupUpdate::Absent));
    }
    let system = system.clone();
    thread::spawn(move || {
        let watched = Udisks::follow(&system, move |_, _| {
            if commands.send(BackupCommand::Open).is_err() {
                tracing::debug!("Settings is closing");
            }
        });
        match watched {
            Ok(udisks) => {
                bande_bus::serve_until_closed(udisks.system());
            }
            Err(error) => tracing::warn!("cannot watch for disks: {error}"),
        }
    });
}

pub fn follow_backup(
    session: &Connection,
    system: &Connection,
    updates: Updates,
) -> Sender<BackupCommand> {
    let session = session.clone();
    let (sender, receiver) = mpsc::channel();
    let following_bus = system.clone();
    let system = crate::follow::waiting_system(system);
    let again = sender.clone();
    spawn("backup", move || {
        let mut following = false;
        while let Ok(command) = receiver.recv() {
            if !following {
                following = true;
                start_following(&following_bus, &updates, again.clone());
            }
            let outcome = BackupProxy::new(&system)
                .and_then(|proxy| run_one(&proxy, &session, &updates, command.clone()));
            if let Err(error) = outcome {
                tracing::warn!("backup: {command:?}: {error}");
            }
        }
    });
    sender
}

fn when(seconds: u64) -> String {
    let formats = DateFormats::from_env();
    let Some(at) = i64::try_from(seconds)
        .ok()
        .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
        .map(|stamp| stamp.to_zoned(local_zone()))
    else {
        return String::new();
    };
    if at.date() == now().date() {
        L10N.tr_args("backup-today-at", [("time", formats.clock(&at).into())])
    } else {
        L10N.tr_args(
            "backup-date-at",
            [
                ("date", formats.date(&at, true).into()),
                ("time", formats.clock(&at).into()),
            ],
        )
    }
}

fn problem_texts(kind: &str) -> Option<(&'static str, &'static str)> {
    match kind {
        problem::OVERDUE => Some(("backup-problem-overdue", "backup-problem-overdue-detail")),
        problem::DISK_FULL => Some((
            "backup-problem-disk-full",
            "backup-problem-disk-full-detail",
        )),
        problem::DAMAGED => Some(("backup-problem-damaged", "backup-problem-damaged-detail")),
        problem::CANNOT_OPEN => Some((
            "backup-problem-cannot-open",
            "backup-problem-cannot-open-detail",
        )),
        problem::FAILING => Some(("backup-problem-failing", "backup-problem-failing-detail")),
        _ => None,
    }
}

impl SettingsView {
    pub(crate) fn backup_opened(&mut self) {
        self.commands.push(Command::Backup(BackupCommand::Open));
    }

    pub(crate) fn backup_updated(&mut self, update: BackupUpdate) {
        match update {
            BackupUpdate::Status(status) => {
                self.backup.absent = false;
                if status.phase != phase::NO_DISK && status.phase != phase::PREPARING {
                    self.backup.choosing = None;
                    self.backup.asking_erase = None;
                    self.backup.network = None;
                    self.backup.first_password = None;
                }
                self.backup.status = Some(status);
            }
            BackupUpdate::Absent => self.backup.absent = true,
            BackupUpdate::Disks(disks) => self.backup.disks = disks,
            BackupUpdate::NetworkChecked(answer) => self.network_checked(&answer),
            BackupUpdate::Inspected(disk, existing) => {
                self.backup.choosing = Some((Target::Disk(disk), existing));
                self.backup.first_password = None;
                self.backup.passwords_differ = false;
                self.focus_next = Some(Anchor::Item(PASSWORD_ANCHOR.to_owned()));
            }
        }
    }

    pub(crate) fn backup_intent(&mut self, action: BackupIntent) {
        match action {
            BackupIntent::Pick(disk) => {
                self.commands
                    .push(Command::Backup(BackupCommand::Inspect(disk)));
            }
            BackupIntent::Password(password) => self.backup_password(password),
            BackupIntent::AskErase(disk) => self.backup.asking_erase = Some(disk),
            BackupIntent::EraseConfirmed => {
                if let Some(disk) = self.backup.asking_erase.take() {
                    self.backup.choosing = Some((Target::Erase(disk), false));
                    self.backup.first_password = None;
                    self.backup.passwords_differ = false;
                    self.focus_next = Some(Anchor::Item(PASSWORD_ANCHOR.to_owned()));
                }
            }
            BackupIntent::StartNetwork => {
                self.backup.network = Some(NetworkDraft {
                    stage: NetworkStage::Address,
                    url: String::new(),
                    user: String::new(),
                    text: String::new(),
                    share_password: None,
                    error: None,
                });
                self.focus_next = Some(Anchor::Item(NETWORK_ANCHOR.to_owned()));
            }
            BackupIntent::NetworkDraft(text) => {
                if let Some(draft) = &mut self.backup.network {
                    draft.text = text;
                }
            }
            BackupIntent::NetworkNext => self.network_next(),
            BackupIntent::NetworkPassword(secret) => {
                if let Some(draft) = &mut self.backup.network {
                    draft.share_password = Some(secret);
                }
                self.check_network();
            }
            BackupIntent::Cancel => {
                self.backup.asking_erase = None;
                self.backup.network = None;
                self.backup.choosing = None;
                self.backup.first_password = None;
                self.backup.passwords_differ = false;
            }
            BackupIntent::Command(command) => {
                self.backup.confirming_stop = false;
                self.commands.push(Command::Backup(command));
            }
            BackupIntent::AskStop(asking) => self.backup.confirming_stop = asking,
        }
    }

    fn backup_password(&mut self, password: Concealed) {
        let Some((target, existing)) = self.backup.choosing.clone() else {
            return;
        };
        self.backup.passwords_differ = false;
        self.focus_next = Some(Anchor::Item(PASSWORD_ANCHOR.to_owned()));
        if !existing {
            match self.backup.first_password.take() {
                None => {
                    self.backup.first_password = Some(password);
                    return;
                }
                Some(first) if first != password => {
                    self.backup.passwords_differ = true;
                    return;
                }
                Some(_) => {}
            }
        }
        let command = match target {
            Target::Disk(disk) => BackupCommand::Choose(disk.uuid, password),
            Target::Erase(disk) => BackupCommand::Erase(disk.block, password),
            Target::Network {
                url,
                user,
                share_password,
            } => BackupCommand::ChooseNetwork(url, user, share_password, password),
        };
        self.commands.push(Command::Backup(command));
    }

    fn network_next(&mut self) {
        let Some(draft) = &mut self.backup.network else {
            return;
        };
        draft.error = None;
        let text = std::mem::take(&mut draft.text);
        match draft.stage {
            NetworkStage::Address => {
                let text = text.trim().to_owned();
                let needs_sign_in = !text.starts_with("nfs://");
                draft.url = text;
                if needs_sign_in {
                    draft.stage = NetworkStage::User;
                    self.focus_next = Some(Anchor::Item(NETWORK_ANCHOR.to_owned()));
                } else {
                    self.check_network();
                }
            }
            NetworkStage::User => {
                let empty = text.is_empty();
                draft.user = text;
                if empty {
                    self.check_network();
                } else {
                    draft.stage = NetworkStage::Password;
                    self.focus_next = Some(Anchor::Item(NETWORK_ANCHOR.to_owned()));
                }
            }
            NetworkStage::Password => {}
        }
    }

    fn check_network(&mut self) {
        let Some(draft) = &self.backup.network else {
            return;
        };
        let secret = draft
            .share_password
            .clone()
            .unwrap_or_else(|| Concealed(Zeroizing::new(String::new())));
        self.commands
            .push(Command::Backup(BackupCommand::CheckNetwork(
                draft.url.clone(),
                draft.user.clone(),
                secret,
            )));
    }

    fn network_checked(&mut self, answer: &str) {
        let Some(draft) = self.backup.network.clone() else {
            return;
        };
        if answer == NEW_PLACE || answer == EXISTING_PLACE {
            self.backup.network = None;
            self.backup.choosing = Some((
                Target::Network {
                    url: draft.url,
                    user: draft.user,
                    share_password: draft
                        .share_password
                        .unwrap_or_else(|| Concealed(Zeroizing::new(String::new()))),
                },
                answer == EXISTING_PLACE,
            ));
            self.backup.first_password = None;
            self.backup.passwords_differ = false;
            self.focus_next = Some(Anchor::Item(PASSWORD_ANCHOR.to_owned()));
            return;
        }
        if let Some(draft) = &mut self.backup.network {
            draft.stage = NetworkStage::Address;
            draft.text = draft.url.clone();
            draft.share_password = None;
            draft.error = Some(if answer == setup_error::BAD_ADDRESS {
                "backup-network-bad-address"
            } else {
                "backup-network-unreachable"
            });
            self.focus_next = Some(Anchor::Item(NETWORK_ANCHOR.to_owned()));
        }
    }

    fn backup_button(&mut self, title: &str, action: BackupIntent) -> Box<dyn Widget> {
        let mut act = intent(&self.intents, move |()| Intent::Backup(action.clone()));
        boxed(Button::new(title, move || act(())))
    }

    pub(crate) fn backup_rows(&mut self) -> Widgets {
        if self.backup.absent {
            return vec![muted_row(&L10N.tr("backup-not-installed"))];
        }
        let Some(status) = self.backup.status.clone() else {
            return vec![muted_row(&L10N.tr("calculating"))];
        };
        if status.phase == phase::PREPARING {
            return vec![boxed(Row::new(
                Text::new(&L10N.tr("backup-preparing"), Tone::Primary),
                Some(boxed(Spinner::new(&L10N.tr("backup-preparing")))),
            ))];
        }
        if let Some((target, existing)) = self.backup.choosing.clone() {
            return vec![self.backup_password_row(&target, existing, &status)];
        }
        if let Some(disk) = self.backup.asking_erase.clone() {
            return vec![self.backup_erase_row(&disk)];
        }
        if let Some(draft) = self.backup.network.clone() {
            return vec![self.backup_network_row(&draft)];
        }
        if status.phase == phase::NO_DISK {
            return self.backup_disk_rows(&status);
        }
        self.backup_status_rows(&status)
    }

    fn backup_disk_rows(&mut self, status: &Status) -> Widgets {
        let mut rows: Widgets = vec![boxed(
            Row::new(Text::new(&L10N.tr("backup-intro"), Tone::Primary), None)
                .detail(Text::new(&L10N.tr("backup-intro-detail"), Tone::Muted).caption()),
        )];
        if status.setup_error == setup_error::DISK_GONE {
            rows.push(muted_row(&L10N.tr("backup-disk-gone")));
        }
        if self.backup.disks.is_empty() {
            rows.push(muted_row(&L10N.tr("backup-no-disks")));
        }
        for disk in self.backup.disks.clone() {
            let detail = L10N.tr_args(
                "backup-disk-detail",
                [
                    ("size", format_size(disk.size).into()),
                    ("filesystem", disk.filesystem.clone().into()),
                ],
            );
            let name = disk.name.clone();
            let erase = self.backup_button(
                &L10N.tr("backup-erase"),
                BackupIntent::AskErase(disk.clone()),
            );
            let mut buttons = vec![erase];
            if !disk.uuid.is_empty() {
                buttons.push(
                    self.backup_button(&L10N.tr("backup-use-disk"), BackupIntent::Pick(disk)),
                );
            }
            let buttons = Stack::new(Axis::Horizontal, Spacing::S as f32, 0.0, buttons);
            rows.push(boxed(
                Row::new(Text::new(&name, Tone::Primary), Some(boxed(buttons)))
                    .detail(Text::new(&detail, Tone::Muted).caption()),
            ));
        }
        let network = self.backup_button(&L10N.tr("backup-network"), BackupIntent::StartNetwork);
        rows.push(boxed(Row::new(Text::new("", Tone::Primary), Some(network))));
        rows
    }

    fn backup_erase_row(&mut self, disk: &DiskChoice) -> Box<dyn Widget> {
        let contents = if disk.contents.is_empty() {
            String::new()
        } else {
            L10N.tr_args(
                "backup-erase-contents",
                [("names", disk.contents.clone().into())],
            )
        };
        let cancel = self.backup_button(&L10N.tr("cancel"), BackupIntent::Cancel);
        let erase = self.backup_button(
            &L10N.tr("backup-erase-confirm"),
            BackupIntent::EraseConfirmed,
        );
        let buttons = Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            vec![cancel, erase],
        );
        boxed(
            Row::new(
                Text::new(
                    &L10N.tr_args(
                        "backup-erase-question",
                        [
                            ("disk", disk.drive_name.clone().into()),
                            ("size", format_size(disk.drive_size).into()),
                        ],
                    ),
                    Tone::Primary,
                ),
                Some(boxed(buttons)),
            )
            .detail(
                Text::new(
                    &L10N.tr_args("backup-erase-warning", [("contents", contents.into())]),
                    Tone::Danger,
                )
                .caption(),
            )
            .highlighted(true),
        )
    }

    fn backup_network_row(&mut self, draft: &NetworkDraft) -> Box<dyn Widget> {
        let cancel = self.backup_button(&L10N.tr("cancel"), BackupIntent::Cancel);
        let (title, detail, control): (String, Option<String>, Box<dyn Widget>) = match draft.stage
        {
            NetworkStage::Password => {
                let mut submit = intent(&self.intents, |password: String| {
                    Intent::Backup(BackupIntent::NetworkPassword(Concealed(Zeroizing::new(
                        password,
                    ))))
                });
                let field =
                    SecureField::new(&L10N.tr("backup-password-field"), FIELD_SIZE, move |text| {
                        submit(text.to_owned());
                    })
                    .with_go_button();
                self.anchors
                    .push((Anchor::Item(NETWORK_ANCHOR.to_owned()), field.id()));
                (L10N.tr("backup-network-password"), None, boxed(field))
            }
            NetworkStage::Address | NetworkStage::User => {
                let mut changed = intent(&self.intents, |text: String| {
                    Intent::Backup(BackupIntent::NetworkDraft(text))
                });
                let (title, detail, placeholder) = if draft.stage == NetworkStage::Address {
                    (
                        L10N.tr("backup-network-address"),
                        None,
                        L10N.tr("backup-network-address-field"),
                    )
                } else {
                    (
                        L10N.tr("backup-network-user"),
                        Some(L10N.tr("backup-network-user-detail")),
                        L10N.tr("backup-network-user-field"),
                    )
                };
                let field = TextField::new(&placeholder, &draft.text, move |text| {
                    changed(text.to_owned());
                });
                self.anchors
                    .push((Anchor::Item(NETWORK_ANCHOR.to_owned()), field.id()));
                let next =
                    self.backup_button(&L10N.tr("backup-network-next"), BackupIntent::NetworkNext);
                let controls = Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![boxed(field), next],
                );
                (title, detail, boxed(controls))
            }
        };
        let controls = Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            vec![control, cancel],
        );
        let mut row = Row::new(Text::new(&title, Tone::Primary), Some(boxed(controls)));
        if let Some(error) = draft.error {
            row = row.detail(Text::new(&L10N.tr(error), Tone::Danger).caption());
        } else if let Some(detail) = detail {
            row = row.detail(Text::new(&detail, Tone::Muted).caption());
        }
        boxed(row.highlighted(true))
    }

    fn backup_excluded_rows(&mut self, status: &Status) -> Widgets {
        let add = self.backup_button(
            &L10N.tr("backup-excluded-add"),
            BackupIntent::Command(BackupCommand::AddExcluded(status.excluded.clone())),
        );
        let mut title = Row::new(
            Text::new(&L10N.tr("backup-excluded"), Tone::Primary),
            Some(add),
        );
        if status.excluded.is_empty() {
            title =
                title.detail(Text::new(&L10N.tr("backup-excluded-none"), Tone::Muted).caption());
        }
        let mut rows: Widgets = vec![boxed(title)];
        for folder in status.excluded.clone() {
            let remaining: Vec<String> = status
                .excluded
                .iter()
                .filter(|known| **known != folder)
                .cloned()
                .collect();
            let remove = self.backup_button(
                &L10N.tr("backup-excluded-remove"),
                BackupIntent::Command(BackupCommand::SetExcluded(remaining)),
            );
            rows.push(boxed(Row::new(
                Text::new(&folder, Tone::Muted),
                Some(remove),
            )));
        }
        rows
    }

    fn backup_password_row(
        &mut self,
        target: &Target,
        existing: bool,
        status: &Status,
    ) -> Box<dyn Widget> {
        let place = match target {
            Target::Disk(disk) => disk.name.clone(),
            Target::Erase(disk) => disk.drive_name.clone(),
            Target::Network { url, .. } => url.clone(),
        };
        let disk_name = [("disk", place.into())];
        let repeating = self.backup.first_password.is_some();
        let (title, detail, placeholder) = if existing {
            (
                L10N.tr_args("backup-existing-password", disk_name),
                L10N.tr("backup-existing-password-detail"),
                L10N.tr("backup-password-field"),
            )
        } else if repeating {
            (
                L10N.tr("backup-repeat-password"),
                L10N.tr("backup-new-password-detail"),
                L10N.tr("backup-repeat-password-field"),
            )
        } else {
            (
                L10N.tr_args("backup-new-password", disk_name),
                L10N.tr("backup-new-password-detail"),
                L10N.tr("backup-password-field"),
            )
        };
        let error = if self.backup.passwords_differ {
            Some(L10N.tr("backup-passwords-differ"))
        } else {
            match status.setup_error.as_str() {
                setup_error::WRONG_PASSWORD => Some(L10N.tr("backup-wrong-password")),
                setup_error::DISK_GONE => Some(L10N.tr("backup-disk-gone")),
                setup_error::FAILED => Some(L10N.tr("backup-setup-failed")),
                setup_error::ERASE_FAILED => Some(L10N.tr("backup-erase-failed")),
                _ => None,
            }
        };
        let mut submit = intent(&self.intents, |password: String| {
            Intent::Backup(BackupIntent::Password(Concealed(Zeroizing::new(password))))
        });
        let field = SecureField::new(&placeholder, FIELD_SIZE, move |password| {
            if !password.is_empty() {
                submit(password.to_owned());
            }
        })
        .with_go_button();
        self.anchors
            .push((Anchor::Item(PASSWORD_ANCHOR.to_owned()), field.id()));
        let cancel = self.backup_button(&L10N.tr("cancel"), BackupIntent::Cancel);
        let controls = Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            vec![boxed(field), cancel],
        );
        let shown_detail = error.as_deref().unwrap_or(&detail);
        let tone = if error.is_some() || !existing {
            Tone::Danger
        } else {
            Tone::Muted
        };
        boxed(
            Row::new(Text::new(&title, Tone::Primary), Some(boxed(controls)))
                .detail(Text::new(shown_detail, tone).caption())
                .highlighted(true),
        )
    }

    fn backup_status_rows(&mut self, status: &Status) -> Widgets {
        let mut rows = Widgets::new();
        if let Some((title, detail)) = problem_texts(&status.problem) {
            rows.push(boxed(
                Row::new(Text::new(&L10N.tr(title), Tone::Danger), None)
                    .detail(Text::new(&L10N.tr(detail), Tone::Muted).caption()),
            ));
        }
        rows.push(self.backup_activity_row(status));
        rows.push(self.backup_disk_row(status));
        rows.extend(self.backup_excluded_rows(status));
        if self.battery.is_some() {
            let mut set = intent(&self.intents, |on: bool| {
                Intent::Backup(BackupIntent::Command(BackupCommand::OnBattery(on)))
            });
            let label = L10N.tr("backup-on-battery");
            rows.push(boxed(Row::new(
                Text::new(&label, Tone::Primary),
                Some(boxed(Toggle::new(&label, status.on_battery, move |on| {
                    set(on);
                }))),
            )));
        }
        if self.backup.confirming_stop {
            rows.push(self.backup_stop_row(status));
        }
        rows
    }

    fn backup_activity_row(&mut self, status: &Status) -> Box<dyn Widget> {
        let last = if status.last_success == 0 {
            L10N.tr("backup-never")
        } else {
            L10N.tr_args("backup-last", [("when", when(status.last_success).into())])
        };
        let (title, detail, control) = match status.phase.as_str() {
            phase::BACKING_UP => {
                let title = L10N.tr("backup-running");
                let minutes = status.remaining_seconds.div_ceil(SECONDS_PER_MINUTE);
                let detail = (minutes > 0)
                    .then(|| L10N.tr_args("backup-remaining", [("minutes", minutes.into())]));
                let bar = ProgressBar::new(&title, Some(status.percent as f32));
                let pause = self.backup_button(
                    &L10N.tr("backup-pause"),
                    BackupIntent::Command(BackupCommand::Pause),
                );
                let controls = Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![boxed(bar), pause],
                );
                (title, detail, Some(boxed(controls) as Box<dyn Widget>))
            }
            phase::TIDYING => (L10N.tr("backup-tidying"), Some(last), None),
            phase::VERIFYING => (L10N.tr("backup-verifying"), Some(last), None),
            phase::PAUSED => {
                let resume = self.backup_button(
                    &L10N.tr("backup-resume"),
                    BackupIntent::Command(BackupCommand::Resume),
                );
                (
                    L10N.tr_args(
                        "backup-paused",
                        [("when", when(status.paused_until).into())],
                    ),
                    Some(last),
                    Some(resume),
                )
            }
            phase::DISK_AWAY if status.on_network => (
                L10N.tr_args(
                    "backup-unreachable",
                    [("disk", status.disk_name.clone().into())],
                ),
                Some(format!("{last}. {}", L10N.tr("backup-unreachable-detail"))),
                None,
            ),
            phase::DISK_AWAY => (
                L10N.tr_args(
                    "backup-disk-away",
                    [("disk", status.disk_name.clone().into())],
                ),
                Some(format!("{last}. {}", L10N.tr("backup-disk-away-detail"))),
                None,
            ),
            phase::WAITING_FOR_POWER => {
                let now_button = self.backup_button(
                    &L10N.tr("backup-now"),
                    BackupIntent::Command(BackupCommand::BackUpNow),
                );
                (
                    L10N.tr("backup-waiting-power"),
                    Some(L10N.tr("backup-waiting-power-detail")),
                    Some(now_button),
                )
            }
            _ => {
                let now_button = self.backup_button(
                    &L10N.tr("backup-now"),
                    BackupIntent::Command(BackupCommand::BackUpNow),
                );
                (last, None, Some(now_button))
            }
        };
        let mut row = Row::new(Text::new(&title, Tone::Primary), control);
        if let Some(detail) = detail {
            row = row.detail(Text::new(&detail, Tone::Muted).caption());
        }
        boxed(row)
    }

    fn backup_disk_row(&mut self, status: &Status) -> Box<dyn Widget> {
        let disk = if status.disk_total > 0 {
            L10N.tr_args(
                "backup-disk-free",
                [
                    ("disk", status.disk_name.clone().into()),
                    ("free", format_size(status.disk_free).into()),
                    ("total", format_size(status.disk_total).into()),
                ],
            )
        } else {
            status.disk_name.clone()
        };
        let stop = self.backup_button(&L10N.tr("backup-stop"), BackupIntent::AskStop(true));
        let mut row = Row::new(
            Text::new(&L10N.tr("backup-disk-row"), Tone::Primary),
            Some(stop),
        );
        let versions = L10N.tr_args("backup-versions", [("count", status.versions.into())]);
        let detail = if status.oldest > 0 {
            let oldest = L10N.tr_args("backup-oldest", [("when", when(status.oldest).into())]);
            format!("{disk}\n{versions}. {oldest}")
        } else {
            disk
        };
        row = row.detail(Text::new(&detail, Tone::Muted).caption());
        boxed(row)
    }

    fn backup_stop_row(&mut self, status: &Status) -> Box<dyn Widget> {
        let cancel = self.backup_button(&L10N.tr("cancel"), BackupIntent::AskStop(false));
        let stop = self.backup_button(
            &L10N.tr("backup-stop-confirm"),
            BackupIntent::Command(BackupCommand::StopUsing),
        );
        let buttons = Stack::new(Axis::Horizontal, Spacing::S as f32, 0.0, vec![cancel, stop]);
        boxed(
            Row::new(
                Text::new(
                    &L10N.tr_args(
                        "backup-stop-question",
                        [("disk", status.disk_name.clone().into())],
                    ),
                    Tone::Primary,
                ),
                Some(boxed(buttons)),
            )
            .detail(Text::new(&L10N.tr("backup-stop-detail"), Tone::Muted).caption())
            .highlighted(true),
        )
    }
}
