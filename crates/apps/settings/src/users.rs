pub use bande_bus::accounts::{suggested_login, valid_login};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use std::sync::mpsc::{self, Sender};

use bande_l10n::{text, tr};
use bande_status::accounts::{ACCOUNTS, ADMINISTRATOR, Accounts, STANDARD, account};
use std::cell::Cell;
use std::rc::Rc;

use crate::crypt::crypted;
use crate::failure::{explained, not_responding};
use crate::follow::{Command, Update, Updates, spawn};
use crate::pages::{FIELD_SIZE, Widgets, boxed, muted_row};
use crate::pictures::choose_picture;
use crate::services::{SETTLE, signals};
use crate::view::{Anchor, Detail, Intent, SettingsView, intent};
use crate::widgets::{Avatar, Card, Header, PictureCrop, Row, Text, Tone};
use bande_settings_panes::{PaneId, SettingId, Value};
use bande_toolkit::ui::{
    Axis, Button, Label, PopUpButton, ScrollState, SecureField, Slider, Stack, TextField, Toggle,
    Widget,
};
use bande_toolkit::{Image, Spacing, TextRole};
use bande_wallpaper::{Fit, render};
use image::RgbaImage;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use zbus::blocking::Connection;
use zbus::{MatchRule, message};
use zeroize::Zeroizing;

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
const PICTURE_TYPES: [&str; 2] = ["image/jpeg", "image/png"];
const PICTURE_PIXELS: u32 = 512;
const PICTURE_QUALITY: u8 = 90;
const AVATAR_PIXELS: u32 = 160;
const WORKING_PIXELS: u32 = 2048;
pub const MAX_ZOOM: f32 = 4.0;
const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_MANAGER: &str = "org.freedesktop.login1.Manager";

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub uid: u32,
    pub login: String,
    pub name: String,
    pub full_name: String,
    pub admin: bool,
    pub you: bool,
    pub logged_in: bool,
    pub picture: Option<Image>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Default for Crop {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 0.5,
            zoom: 1.0,
        }
    }
}

impl Crop {
    pub fn side(self, width: u32, height: u32) -> f32 {
        width.min(height) as f32 / self.zoom
    }

    pub fn clamped(self, width: u32, height: u32) -> Self {
        let zoom = self.zoom.clamp(1.0, MAX_ZOOM);
        let half = width.min(height) as f32 / zoom / 2.0;
        let along = |centre: f32, length: u32| {
            let length = length.max(1) as f32;
            (centre * length).clamp(half, (length - half).max(half)) / length
        };
        Self {
            x: along(self.x, width),
            y: along(self.y, height),
            zoom,
        }
    }

    pub fn square(self, width: u32, height: u32) -> (u32, u32, u32) {
        let crop = self.clamped(width, height);
        let side = crop.side(width, height);
        let origin = |centre: f32, length: u32| {
            ((centre * length as f32 - side / 2.0).round().max(0.0) as u32)
                .min(length.saturating_sub(side.round() as u32))
        };
        (
            origin(crop.x, width),
            origin(crop.y, height),
            (side.round() as u32).max(1),
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NewAccount {
    pub full_name: String,
    pub login: String,
    pub admin: bool,
    pub password: Zeroizing<String>,
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
                uid: id,
                login: (*login).to_owned(),
                name: if name.is_empty() { *login } else { name }.to_owned(),
                full_name: name.to_owned(),
                admin: admins.iter().any(|admin| admin == login),
                you: id == uid,
                logged_in: false,
                picture: None,
            })
        })
        .collect();
    sort(&mut accounts);
    accounts
}

fn sort(accounts: &mut [Account]) {
    accounts.sort_by_key(|account| (!account.you, account.name.to_lowercase()));
}

fn avatar(path: &Path) -> Option<Image> {
    let picture = render(path, Fit::Fill, AVATAR_PIXELS, AVATAR_PIXELS)
        .inspect_err(|error| tracing::info!("{}: {error}", path.display()))
        .ok()?;
    Image::from_rgba(picture.width, picture.height, picture.rgba)
}

fn described(system: &Connection, account: &mut Account) -> zbus::Result<()> {
    let user = bande_status::accounts::account(system, &account.login)?;
    let full_name = user.real_name()?;
    account.name = match full_name.trim() {
        "" => account.login.clone(),
        name => name.to_owned(),
    };
    account.full_name = full_name;
    account.admin = user.account_type()? == ADMINISTRATOR;
    let icon = PathBuf::from(user.icon_file()?);
    account.picture = icon.is_absolute().then(|| avatar(&icon)).flatten();
    Ok(())
}

fn logged_in(system: &Connection) -> zbus::Result<Vec<u32>> {
    let reply = system.call_method(
        Some(LOGIN1),
        "/org/freedesktop/login1",
        Some(LOGIN1_MANAGER),
        "ListUsers",
        &(),
    )?;
    let users: Vec<(u32, String, zbus::zvariant::OwnedObjectPath)> = reply.body().deserialize()?;
    Ok(users.into_iter().map(|(uid, ..)| uid).collect())
}

fn refuse_logged_in(system: &Connection, uid: u32) -> Result<(), String> {
    let users = logged_in(system).map_err(|error| {
        explained(
            &error,
            text!("The login service"),
            text!("Couldn't change this account."),
        )
    })?;
    if !users.contains(&uid) {
        return Ok(());
    }
    let name = accounts()
        .into_iter()
        .find(|account| account.uid == uid)
        .map_or_else(|| uid.to_string(), |account| account.name);
    Err(tr!(
        "{name} is logged in. To delete this user, they must log out first.",
        name = name
    ))
}

pub fn managed_accounts(system: Option<&Connection>) -> (Vec<Account>, bool) {
    let mut listed = accounts();
    let Some(system) = system else {
        return (listed, false);
    };
    match logged_in(system) {
        Ok(users) => {
            for account in &mut listed {
                account.logged_in = users.contains(&account.uid);
            }
        }
        Err(error) => tracing::warn!("logind: {error}"),
    }
    for account in &mut listed {
        if let Err(error) = described(system, account) {
            tracing::warn!("AccountsService: {error}");
            return (accounts(), false);
        }
    }
    sort(&mut listed);
    (listed, true)
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
    let first = session.wait_for(&[CURRENT_PROMPT, LEGACY_CURRENT_PROMPT, NEW_PROMPT]);
    if first.is_none() {
        return PasswordOutcome::Failed(session.message());
    }
    if matches!(first, Some(0 | 1)) {
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

#[derive(Clone, PartialEq)]
pub enum UsersCommand {
    Refresh,
    ChangePassword(Zeroizing<String>, Zeroizing<String>),
    ChoosePicture(String),
    SavePicture(String, Crop),
    Rename(String, String),
    Administrator(String, bool),
    Create(NewAccount),
    Delete(u32, bool),
}

impl std::fmt::Debug for UsersCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refresh => f.write_str("Refresh"),
            Self::ChangePassword(..) => f.write_str("ChangePassword"),
            Self::ChoosePicture(login) => write!(f, "ChoosePicture({login})"),
            Self::SavePicture(login, crop) => write!(f, "SavePicture({login}, {crop:?})"),
            Self::Rename(login, name) => write!(f, "Rename({login}, {name})"),
            Self::Administrator(login, on) => write!(f, "Administrator({login}, {on})"),
            Self::Create(new) => write!(f, "Create({}, admin: {})", new.login, new.admin),
            Self::Delete(uid, remove) => write!(f, "Delete({uid}, remove files: {remove})"),
        }
    }
}

fn working_copy(path: &Path) -> Result<RgbaImage, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let decoded = image::load_from_memory(&bytes)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    let longest = width.max(height);
    if width == 0 || height == 0 {
        return Err("the picture has no pixels".to_owned());
    }
    if longest <= WORKING_PIXELS {
        return Ok(decoded);
    }
    let scale = f64::from(WORKING_PIXELS) / f64::from(longest);
    let scaled = |side: u32| ((f64::from(side) * scale).round() as u32).max(1);
    Ok(imageops::resize(
        &decoded,
        scaled(width),
        scaled(height),
        FilterType::CatmullRom,
    ))
}

fn cropped(source: &RgbaImage, crop: Crop) -> Result<Vec<u8>, String> {
    let (x, y, side) = crop.square(source.width(), source.height());
    let square = imageops::crop_imm(source, x, y, side, side).to_image();
    let picture = imageops::resize(
        &square,
        PICTURE_PIXELS,
        PICTURE_PIXELS,
        FilterType::Lanczos3,
    );
    let rgb = image::DynamicImage::ImageRgba8(picture).to_rgb8();
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, PICTURE_QUALITY)
        .encode_image(&rgb)
        .map_err(|error| error.to_string())?;
    Ok(encoded)
}

fn set_picture(system: &Connection, login: &str, encoded: &[u8]) -> Result<(), String> {
    let folder = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let file = tempfile_in(&folder, encoded).map_err(|error| error.to_string())?;
    let result = account(system, login)
        .and_then(|user| user.set_icon_file(&file.to_string_lossy()))
        .map_err(|error| {
            explained(
                &error,
                text!("The accounts service"),
                text!("Couldn't change this account."),
            )
        });
    if let Err(error) = std::fs::remove_file(&file) {
        tracing::warn!("cannot remove {}: {error}", file.display());
    }
    result
}

fn tempfile_in(folder: &Path, contents: &[u8]) -> io::Result<PathBuf> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = folder.join(format!("bande-settings-picture-{}.jpg", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(contents)?;
    Ok(path)
}

fn create(system: &Connection, new: &NewAccount) -> Result<(), String> {
    let refused = |error: zbus::Error| {
        explained(
            &error,
            text!("The accounts service"),
            text!("Couldn't change this account."),
        )
    };
    let crypted = crypted(&new.password)?;
    let kind = if new.admin { ADMINISTRATOR } else { STANDARD };
    let path = Accounts::new(system)
        .and_then(|accounts| accounts.create_user(&new.login, new.full_name.trim(), kind))
        .map_err(refused)?;
    bande_status::accounts::AccountUser::builder(system)
        .path(path)
        .and_then(|builder| builder.build())
        .and_then(|user| user.set_password(&crypted, ""))
        .map_err(refused)
}

fn follow_accounts(system: Option<Connection>, updates: Updates) -> Sender<()> {
    let (changed, changes) = mpsc::channel::<()>();
    let watch = changed.clone();
    spawn("users", move || {
        let mut watching = false;
        while changes.recv().is_ok() {
            std::thread::sleep(SETTLE);
            changes.try_iter().count();
            if !watching && let Some(system) = &system {
                watching = true;
                if let Ok(rule) = MatchRule::builder()
                    .msg_type(message::Type::Signal)
                    .sender(ACCOUNTS)
                    .map(|builder| builder.build())
                {
                    signals(system, rule, watch.clone(), |_| Some(()), || true);
                }
                for member in ["UserNew", "UserRemoved"] {
                    if let Ok(rule) = MatchRule::builder()
                        .msg_type(message::Type::Signal)
                        .sender(LOGIN1)
                        .and_then(|builder| builder.interface(LOGIN1_MANAGER))
                        .and_then(|builder| builder.member(member))
                        .map(|builder| builder.build())
                    {
                        signals(system, rule, watch.clone(), |_| Some(()), || true);
                    }
                }
            }
            let (accounts, managed) = managed_accounts(system.as_ref());
            if !updates.send(Update::Accounts(accounts, managed)) {
                break;
            }
        }
    });
    changed
}

pub fn follow_users(
    session: &Connection,
    system: Option<&Connection>,
    updates: Updates,
) -> Sender<UsersCommand> {
    let (sender, receiver) = mpsc::channel();
    let changed = follow_accounts(system.cloned(), updates.clone());
    let session = session.clone();
    let system = system.map(crate::follow::waiting_system);
    spawn("users-worker", move || {
        let mut chosen: Option<(String, RgbaImage)> = None;
        for command in receiver {
            let result = match command {
                UsersCommand::Refresh => Ok(()),
                UsersCommand::ChangePassword(current, new) => {
                    updates.send(Update::Password(change_password(&current, &new)));
                    Ok(())
                }
                UsersCommand::ChoosePicture(login) => {
                    match choose_picture(&session, &tr!("Choose a Picture"), &PICTURE_TYPES) {
                        Ok(Some(path)) => {
                            let shown = working_copy(&path).and_then(|copy| {
                                let shown =
                                    Image::from_rgba(copy.width(), copy.height(), copy.to_vec())
                                        .ok_or_else(|| "the picture is too large".to_owned())?;
                                Ok((copy, shown))
                            });
                            match shown {
                                Ok((copy, shown)) => {
                                    chosen = Some((login.clone(), copy));
                                    updates.send(Update::PictureChosen(login, shown));
                                    Ok(())
                                }
                                Err(error) => {
                                    tracing::warn!("{}: {error}", path.display());
                                    Err(tr!(
                                        "That picture can't be opened. Choose a JPEG or PNG picture."
                                    ))
                                }
                            }
                        }
                        Ok(None) => Ok(()),
                        Err(error) => Err(explained(
                            &error,
                            text!("The file chooser"),
                            text!("Couldn't open the file chooser."),
                        )),
                    }
                }
                UsersCommand::SavePicture(login, crop) => match (&system, chosen.take()) {
                    (Some(system), Some((owner, copy))) if owner == login => cropped(&copy, crop)
                        .and_then(|encoded| set_picture(system, &login, &encoded)),
                    _ => Ok(()),
                },
                UsersCommand::Rename(login, name) => system.as_ref().map_or(Ok(()), |system| {
                    account(system, &login)
                        .and_then(|user| user.set_real_name(name.trim()))
                        .map_err(|error| {
                            explained(
                                &error,
                                text!("The accounts service"),
                                text!("Couldn't change this account."),
                            )
                        })
                }),
                UsersCommand::Administrator(login, on) => {
                    system.as_ref().map_or(Ok(()), |system| {
                        account(system, &login)
                            .and_then(|user| {
                                user.set_account_type(if on { ADMINISTRATOR } else { STANDARD })
                            })
                            .map_err(|error| {
                                explained(
                                    &error,
                                    text!("The accounts service"),
                                    text!("Couldn't change this account."),
                                )
                            })
                    })
                }
                UsersCommand::Create(new) => {
                    let created = system.as_ref().map_or_else(
                        || Err(not_responding(text!("The accounts service"))),
                        |system| create(system, &new),
                    );
                    updates.send(Update::AccountCreated(created.clone().err()));
                    created.map_err(|_| String::new())
                }
                UsersCommand::Delete(uid, remove_files) => {
                    system.as_ref().map_or(Ok(()), |system| {
                        refuse_logged_in(system, uid)?;
                        Accounts::new(system)
                            .and_then(|accounts| accounts.delete_user(i64::from(uid), remove_files))
                            .map_err(|error| {
                                explained(
                                    &error,
                                    text!("The accounts service"),
                                    text!("Couldn't change this account."),
                                )
                            })
                    })
                }
            };
            match result {
                Err(message) if !message.is_empty() => {
                    if !updates.send(Update::PaneFailed(PaneId::Users, message)) {
                        break;
                    }
                }
                _ => {}
            }
            let _ = changed.send(());
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
const PASSWORD_LABELS: [bande_l10n::Text; 3] = [
    text!("Current password"),
    text!("New password"),
    text!("Verify"),
];
const NEW_PASSWORD_FIELDS: [&str; 2] = ["password-account-new", "password-account-verify"];
const NEW_NAME: &str = "new-account-name";
const NEW_LOGIN: &str = "new-account-login";
const DELETE_CONFIRM: &str = "delete-user-confirm";

#[derive(Clone)]
pub(crate) struct PictureEdit {
    pub login: String,
    pub image: Image,
    pub crop: Rc<Cell<Crop>>,
}

#[derive(Clone, Default)]
pub(crate) struct NewAccountForm {
    pub full_name: String,
    pub login: String,
    pub login_edited: bool,
    pub admin: bool,
    pub password: Option<Zeroizing<String>>,
    pub busy: bool,
    pub error: Option<String>,
}

impl SettingsView {
    fn account_detail(account: &Account) -> String {
        let kind = if account.admin {
            tr!("Administrator")
        } else {
            tr!("Standard")
        };
        format!("{} · {kind}", account.login)
    }

    fn account_title(account: &Account) -> String {
        if account.you {
            tr!("{name} (you)", name = account.name.clone())
        } else {
            account.name.clone()
        }
    }

    pub(crate) fn account_rows(&mut self) -> Widgets {
        let Some(accounts) = self.accounts.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for account in accounts {
            let mut open = intent(&self.intents, Intent::OpenAccount);
            let login = account.login.clone();
            let row = Row::new(
                Text::new(&Self::account_title(&account), Tone::Primary),
                Some(boxed(Label::new("›"))),
            )
            .avatar(Avatar::new(account.picture.clone(), &account.name))
            .detail(Text::new(&Self::account_detail(&account), Tone::Muted).caption())
            .on_activate(move || open(login.clone()));
            rows.push(self.item_row(format!("account:{}", account.login), row));
        }
        if self.accounts_managed {
            match self.new_account.clone() {
                Some(form) => rows.extend(self.new_account_rows(&form)),
                None => {
                    let mut ask = intent(&self.intents, |()| Intent::AskNewAccount(true));
                    let button = Button::new(&tr!("Add User…"), move || ask(()));
                    self.anchors
                        .push((Anchor::Item("add-user".to_owned()), button.id()));
                    rows.push(boxed(Row::new(
                        Text::new(
                            &tr!("Add a person who can log in to this computer"),
                            Tone::Primary,
                        ),
                        Some(boxed(button)),
                    )));
                }
            }
        }
        rows
    }

    fn new_account_rows(&mut self, form: &NewAccountForm) -> Widgets {
        let mut rows = Vec::new();
        let mut name = intent(&self.intents, Intent::NewAccountName);
        let field = TextField::new(&tr!("Full name"), &form.full_name, move |text| {
            name(text.to_owned())
        })
        .without_text_checking();
        self.anchors
            .push((Anchor::Item(NEW_NAME.to_owned()), field.id()));
        rows.push(boxed(Row::new(
            Text::new(&tr!("Full name"), Tone::Primary),
            Some(boxed(field)),
        )));
        let mut login = intent(&self.intents, Intent::NewAccountLogin);
        let field = TextField::new(&tr!("Account name"), &form.login, move |text| {
            login(text.to_owned())
        })
        .without_text_checking();
        self.anchors
            .push((Anchor::Item(NEW_LOGIN.to_owned()), field.id()));
        rows.push(boxed(
            Row::new(
                Text::new(&tr!("Account name"), Tone::Primary),
                Some(boxed(field)),
            )
            .detail(
                Text::new(
                    &tr!("The name of the home folder; it can't be changed later."),
                    Tone::Muted,
                )
                .caption(),
            ),
        ));
        let kinds = [tr!("Standard"), tr!("Administrator")];
        let mut admin = intent(&self.intents, Intent::NewAccountAdmin);
        let menu = PopUpButton::new(
            &tr!("Account type"),
            &kinds,
            usize::from(form.admin),
            move |index| admin(index == 1),
        );
        self.anchors
            .push((Anchor::Item("new-account-type".to_owned()), menu.id()));
        rows.push(boxed(Row::new(
            Text::new(&tr!("Account type"), Tone::Primary),
            Some(boxed(menu)),
        )));
        for (step, anchor) in NEW_PASSWORD_FIELDS.iter().enumerate() {
            let label = if step == 0 {
                tr!("Password")
            } else {
                tr!("Verify")
            };
            let mut entry = intent(&self.intents, move |text: String| {
                Intent::NewAccountPassword(step, text)
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
            (_, true) => tr!("Creating the account…"),
            (Some(error), false) => error.clone(),
            (None, false) => tr!(
                "Press Return after each password field to create the account, or Escape to cancel"
            ),
        };
        let mut cancel = intent(&self.intents, |()| Intent::AskNewAccount(false));
        rows.push(boxed(
            Row::new(
                Text::new(&detail, Tone::Muted).caption(),
                Some(boxed(
                    Button::new(&tr!("Cancel"), move || cancel(())).cancel(),
                )),
            )
            .highlighted(form.error.is_some()),
        ));
        rows
    }

    pub(crate) fn account_page(&mut self) -> Option<Widgets> {
        let Some(Detail::Account(login)) = self.detail.clone() else {
            return None;
        };
        let account = self
            .accounts
            .as_ref()?
            .iter()
            .find(|account| account.login == login)?
            .clone();
        let managed = self.accounts_managed;
        let mut page: Widgets = self.back_button(PaneId::Users).into_iter().collect();
        page.push(boxed(
            Header::new(
                None,
                Text::new(&Self::account_title(&account), Tone::Primary).role(TextRole::Title),
                &Self::account_detail(&account),
                None,
            )
            .avatar(Avatar::new(account.picture.clone(), &account.name)),
        ));
        if managed {
            let mut rows = self.account_picture_rows(&account);
            rows.push(self.manual_field(
                SettingId::FullName,
                text!("Full name"),
                text!("Full name"),
            ));
            let control: Box<dyn Widget> = if account.you {
                boxed(Label::new(&if account.admin {
                    tr!("Administrator")
                } else {
                    tr!("Standard")
                }))
            } else {
                let mut set = intent(&self.intents, |(login, on)| {
                    Intent::Administrator(login, on)
                });
                let target = login.clone();
                let toggle = Toggle::new(
                    &tr!("Allow this user to administer this computer"),
                    account.admin,
                    move |on| set((target.clone(), on)),
                );
                self.anchors
                    .push((Anchor::Item("account-administer".to_owned()), toggle.id()));
                boxed(toggle)
            };
            rows.push(boxed(Row::new(
                Text::new(
                    &tr!("Allow this user to administer this computer"),
                    Tone::Primary,
                ),
                Some(control),
            )));
            page.push(boxed(Card::new(rows)));
        }
        if account.you {
            let mut rows = Vec::new();
            if self.password.is_none() {
                let mut ask = intent(&self.intents, |()| Intent::AskPassword(true));
                let button = Button::new(&tr!("Change Password…"), move || ask(()));
                self.anchors
                    .push((Anchor::Item("change-password".to_owned()), button.id()));
                rows.push(boxed(Row::new(
                    Text::new(&tr!("Password"), Tone::Primary),
                    Some(boxed(button)),
                )));
            } else {
                rows.extend(self.password_rows());
            }
            if let Some(message) = &self.password_message {
                rows.push(muted_row(message));
            }
            page.push(boxed(Card::new(rows)));
        } else if managed {
            page.push(boxed(Card::new(vec![self.delete_account_row(&account)])));
        }
        Some(page)
    }

    fn account_picture_rows(&mut self, account: &Account) -> Widgets {
        let Some(edit) = self
            .picture_edit
            .clone()
            .filter(|edit| edit.login == account.login)
        else {
            let mut choose = intent(&self.intents, |()| Intent::ChooseAccountPicture);
            let button = Button::new(&tr!("Choose…"), move || choose(()));
            self.anchors
                .push((Anchor::Item("account-picture".to_owned()), button.id()));
            return vec![boxed(Row::new(
                Text::new(&tr!("Picture"), Tone::Primary),
                Some(boxed(button)),
            ))];
        };
        let mut moved = intent(&self.intents, |()| Intent::PictureMoved);
        let crop = PictureCrop::new(
            &tr!("Picture, drag to move and scroll to zoom"),
            edit.image.clone(),
            Rc::clone(&edit.crop),
            move || moved(()),
        );
        self.anchors
            .push((Anchor::Item("account-picture-crop".to_owned()), crop.id()));
        let shared = Rc::clone(&edit.crop);
        let zoom = Slider::new(
            &tr!("Zoom"),
            f64::from(edit.crop.get().zoom),
            (1.0, f64::from(MAX_ZOOM)),
            0.0,
            move |zoom| {
                shared.set(Crop {
                    zoom: zoom as f32,
                    ..shared.get()
                })
            },
        );
        let mut cancel = intent(&self.intents, |()| Intent::KeepPicture(false));
        let mut save = intent(&self.intents, |()| Intent::KeepPicture(true));
        let save = Button::new(&tr!("Save"), move || save(()));
        self.anchors
            .push((Anchor::Item("account-picture-save".to_owned()), save.id()));
        vec![
            boxed(crop),
            boxed(Row::new(
                Text::new(&tr!("Zoom"), Tone::Primary),
                Some(boxed(zoom)),
            )),
            boxed(Row::new(
                Text::new(
                    &tr!("Drag the picture to choose what shows in the circle."),
                    Tone::Muted,
                )
                .caption(),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![
                        boxed(Button::new(&tr!("Cancel"), move || cancel(())).cancel()),
                        boxed(save.primary()),
                    ],
                ))),
            )),
        ]
    }

    fn delete_account_row(&mut self, account: &Account) -> Box<dyn Widget> {
        if !self.deleting_account || account.logged_in {
            let mut ask = intent(&self.intents, |()| Intent::AskDeleteAccount(true));
            let mut button = Button::new(&tr!("Delete User…"), move || ask(()));
            button.set_enabled(!account.logged_in);
            self.anchors
                .push((Anchor::Item("delete-user".to_owned()), button.id()));
            let row = Row::new(
                Text::new(&tr!("Delete this user"), Tone::Primary),
                Some(boxed(button)),
            );
            return boxed(if account.logged_in {
                row.detail(
                    Text::new(
                        &tr!(
                            "{name} is logged in. To delete this user, they must log out first.",
                            name = account.name.clone()
                        ),
                        Tone::Muted,
                    )
                    .caption(),
                )
            } else {
                row
            });
        }
        let uid = account.uid;
        let mut cancel = intent(&self.intents, |()| Intent::AskDeleteAccount(false));
        let mut keep = intent(&self.intents, move |()| Intent::DeleteAccount(uid, false));
        let mut remove = intent(&self.intents, move |()| Intent::DeleteAccount(uid, true));
        let keep = Button::new(&tr!("Keep Home Folder"), move || keep(()));
        self.anchors
            .push((Anchor::Item(DELETE_CONFIRM.to_owned()), keep.id()));
        boxed(
            Row::new(
                Text::new(
                    &tr!("Delete “{name}”?", name = account.name.clone()),
                    Tone::Primary,
                ),
                Some(boxed(Stack::new(
                    Axis::Horizontal,
                    Spacing::S as f32,
                    0.0,
                    vec![
                        boxed(Button::new(&tr!("Cancel"), move || cancel(())).cancel()),
                        boxed(keep),
                        boxed(
                            Button::new(&tr!("Delete Home Folder"), move || remove(()))
                                .primary(),
                        ),
                    ],
                ))),
            )
            .detail(
                Text::new(
                    &tr!("Keep the home folder to keep their files in /home/{login}, or delete it and everything in it.", login = account.login.clone()),
                    Tone::Muted,
                )
                .caption(),
            )
            .highlighted(true),
        )
    }

    pub(crate) fn take_accounts(&mut self, accounts: Vec<Account>, managed: bool) {
        if let Some(Detail::Account(login)) = &self.detail {
            match accounts.iter().find(|account| account.login == *login) {
                Some(account) => {
                    self.values
                        .insert(SettingId::FullName, Value::Text(account.full_name.clone()));
                }
                None => {
                    self.detail = None;
                    self.deleting_account = false;
                    self.focus_next = Some(Anchor::Setting(SettingId::Accounts, None));
                }
            }
        }
        self.accounts = Some(accounts);
        self.accounts_managed = managed;
    }

    pub(crate) fn open_account(&mut self, login: String) {
        if let Some(account) = self
            .accounts
            .iter()
            .flatten()
            .find(|account| account.login == login)
        {
            self.values
                .insert(SettingId::FullName, Value::Text(account.full_name.clone()));
        }
        self.drafts.remove(&SettingId::FullName);
        self.clear_failure();
        self.password = None;
        self.password_message = None;
        self.picture_edit = None;
        self.deleting_account = false;
        self.detail = Some(Detail::Account(login));
        self.focus_next = Some(Anchor::Back);
        *self.scroll.borrow_mut() = ScrollState::default();
    }

    fn shown_account(&self) -> Option<String> {
        match &self.detail {
            Some(Detail::Account(login)) => Some(login.clone()),
            _ => None,
        }
    }

    pub(crate) fn choose_account_picture(&mut self) {
        if let Some(login) = self.shown_account() {
            self.clear_failure();
            self.commands
                .push(Command::Users(UsersCommand::ChoosePicture(login)));
        }
    }

    pub(crate) fn picture_chosen(&mut self, login: String, image: Image) {
        if self.shown_account().as_ref() != Some(&login) {
            return;
        }
        self.picture_edit = Some(PictureEdit {
            login,
            image,
            crop: Rc::default(),
        });
        self.focus_next = Some(Anchor::Item("account-picture-crop".to_owned()));
    }

    pub(crate) fn keep_picture(&mut self, keep: bool) {
        let Some(edit) = self.picture_edit.take() else {
            return;
        };
        if keep {
            let crop = edit
                .crop
                .get()
                .clamped(edit.image.width(), edit.image.height());
            self.commands
                .push(Command::Users(UsersCommand::SavePicture(edit.login, crop)));
        }
        self.focus_next = Some(Anchor::Item("account-picture".to_owned()));
    }

    pub(crate) fn rename_account(&mut self, value: Value) {
        let (Some(login), Value::Text(name)) = (self.shown_account(), &value) else {
            return;
        };
        if name.trim().is_empty() {
            return;
        }
        self.commands
            .push(Command::Users(UsersCommand::Rename(login, name.clone())));
        self.values.insert(SettingId::FullName, value);
    }

    pub(crate) fn set_administrator(&mut self, login: String, on: bool) {
        self.clear_failure();
        self.commands
            .push(Command::Users(UsersCommand::Administrator(login, on)));
    }

    pub(crate) fn ask_delete_account(&mut self, asking: bool) {
        self.deleting_account = asking;
        self.focus_next = Some(Anchor::Item(
            if asking {
                DELETE_CONFIRM
            } else {
                "delete-user"
            }
            .to_owned(),
        ));
    }

    pub(crate) fn delete_account(&mut self, uid: u32, remove_files: bool) {
        self.deleting_account = false;
        self.clear_failure();
        self.commands
            .push(Command::Users(UsersCommand::Delete(uid, remove_files)));
    }

    pub(crate) fn ask_new_account(&mut self, asking: bool) {
        if self.new_account.as_ref().is_some_and(|form| form.busy) {
            return;
        }
        self.new_account = asking.then(NewAccountForm::default);
        self.focus_next = Some(Anchor::Item(
            if asking { NEW_NAME } else { "add-user" }.to_owned(),
        ));
    }

    pub(crate) fn new_account_field(&mut self, intent: Intent) {
        let Some(form) = self.new_account.as_mut().filter(|form| !form.busy) else {
            return;
        };
        let mut suggestion = None;
        match intent {
            Intent::NewAccountName(name) => {
                let suggested = suggested_login(&name);
                form.full_name = name;
                if !form.login_edited && form.login != suggested {
                    form.login.clone_from(&suggested);
                    suggestion = Some(suggested);
                }
            }
            Intent::NewAccountLogin(login) => {
                form.login_edited = login != suggested_login(&form.full_name);
                form.login = login;
            }
            Intent::NewAccountAdmin(admin) => form.admin = admin,
            _ => {}
        }
        let field = self
            .anchors
            .iter()
            .find(|(anchor, _)| *anchor == Anchor::Item(NEW_LOGIN.to_owned()))
            .map(|(_, id)| *id);
        if let (Some(suggested), Some(field)) = (suggestion, field)
            && let Some(field) = self.ui().widget_as::<TextField>(field)
        {
            field.replace_text(&suggested);
        }
    }

    pub(crate) fn new_account_password(&mut self, step: usize, text: String) {
        let existing: Vec<String> = self
            .accounts
            .iter()
            .flatten()
            .map(|account| account.login.clone())
            .collect();
        let Some(form) = self.new_account.as_mut().filter(|form| !form.busy) else {
            return;
        };
        let text = Zeroizing::new(text);
        let login = form.login.trim().to_owned();
        let problem = if form.full_name.trim().is_empty() {
            Some((text!("Type the person's full name."), NEW_NAME))
        } else if !valid_login(&login) {
            Some((
                text!(
                    "Use lowercase letters, digits, - and _ for the account name, starting with a letter."
                ),
                NEW_LOGIN,
            ))
        } else if existing.contains(&login) {
            Some((
                text!("That account name is already used. Choose another one."),
                NEW_LOGIN,
            ))
        } else {
            None
        };
        let next = match step {
            0 if text.is_empty() => {
                form.error = Some(tr!("Type a new password."));
                NEW_PASSWORD_FIELDS[0]
            }
            0 => {
                form.password = Some(text);
                form.error = None;
                NEW_PASSWORD_FIELDS[1]
            }
            _ => match (&form.password, problem) {
                (None, _) => {
                    form.error = Some(tr!("Type a new password."));
                    NEW_PASSWORD_FIELDS[0]
                }
                (Some(password), _) if *password != text => {
                    form.password = None;
                    form.error = Some(tr!("The passwords don't match. Type the password again."));
                    NEW_PASSWORD_FIELDS[0]
                }
                (Some(_), Some((error, field))) => {
                    form.error = Some(error.tr());
                    field
                }
                (Some(password), None) => {
                    form.busy = true;
                    form.error = None;
                    self.commands
                        .push(Command::Users(UsersCommand::Create(NewAccount {
                            full_name: form.full_name.trim().to_owned(),
                            login,
                            admin: form.admin,
                            password: password.clone(),
                        })));
                    return;
                }
            },
        };
        self.focus_next = Some(Anchor::Item(next.to_owned()));
    }

    pub(crate) fn account_created(&mut self, error: Option<String>) {
        let Some(form) = self.new_account.as_mut() else {
            return;
        };
        form.busy = false;
        match error {
            None => {
                self.new_account = None;
                self.focus_next = Some(Anchor::Item("add-user".to_owned()));
            }
            Some(error) => {
                form.password = None;
                form.error = Some(error);
                self.focus_next = Some(Anchor::Item(NEW_PASSWORD_FIELDS[0].to_owned()));
            }
        }
    }

    fn password_rows(&mut self) -> Widgets {
        let Some(form) = self.password.clone() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        for (step, (anchor, label)) in PASSWORD_FIELDS.iter().zip(PASSWORD_LABELS).enumerate() {
            let label = label.tr();
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
            (_, true) => tr!("Changing the password…"),
            (Some(error), false) => error.clone(),
            (None, false) => {
                tr!("Press Return after each field to change the password, or Escape to cancel")
            }
        };
        let mut cancel = intent(&self.intents, |()| Intent::AskPassword(false));
        rows.push(boxed(Row::new(
            Text::new(&detail, Tone::Muted).caption(),
            Some(boxed(
                Button::new(&tr!("Cancel"), move || cancel(())).cancel(),
            )),
        )));
        rows
    }

    pub(crate) fn ask_password(&mut self, asking: bool) {
        if self.password.as_ref().is_some_and(|form| form.busy) {
            return;
        }
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
                form.error = Some(tr!("Type a new password."));
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
                    form.error = Some(tr!("Type your current password first."));
                    Some(0)
                }
                _ => {
                    form.new = None;
                    form.error = Some(tr!(
                        "The new passwords don't match. Type the new password again."
                    ));
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
            if outcome == PasswordOutcome::Changed {
                self.password_message = Some(tr!(
                    "Your password was changed. Use it the next time you log in or unlock."
                ));
            }
            return;
        };
        form.busy = false;
        let (error, field) = match outcome {
            PasswordOutcome::Changed => {
                self.password = None;
                self.password_message = Some(tr!(
                    "Your password was changed. Use it the next time you log in or unlock."
                ));
                self.focus_next = Some(Anchor::Item("change-password".to_owned()));
                return;
            }
            PasswordOutcome::WrongPassword => {
                form.current = None;
                (tr!("The current password is not correct."), 0)
            }
            PasswordOutcome::Rejected(reason) if !reason.is_empty() => {
                form.new = None;
                (
                    tr!("That password can't be used: {reason}", reason = reason),
                    1,
                )
            }
            PasswordOutcome::Rejected(_) => {
                form.new = None;
                (tr!("That password is too weak. Choose a longer one."), 1)
            }
            PasswordOutcome::Failed(reason) => {
                tracing::warn!("passwd: {reason}");
                (tr!("Couldn't change the password."), 0)
            }
        };
        form.error = Some(error);
        self.focus_next = Some(Anchor::Item(PASSWORD_FIELDS[field].to_owned()));
    }
}
