use std::fs;
use std::path::Path;

use bande_watch::{Changes, FolderWatch};
use zbus::blocking::Connection;

use crate::L10N;
use crate::follow::{Update, Updates};
use crate::pages::{Widgets, boxed};
use crate::view::SettingsView;
use crate::widgets::{Row, Text, Tone};

const DISK_DIRECTORY: &str = "/run/bande-disk-encryption";
const DISK_STATUS: &str = "/run/bande-disk-encryption/status";
const DISK_PROGRESS: &str = "/run/bande-disk-encryption/progress";
const REMOTE_LOGIN_UNIT: &str = "ssh.service";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DiskEncryption {
    #[default]
    Plain,
    Off,
    Encrypting(u8),
    On,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SecurityStatus {
    pub disk: DiskEncryption,
    pub remote_login: bool,
}

fn percent_done(progress: &str) -> Option<u8> {
    let last = progress.lines().next_back()?;
    let number = |field: &str| -> Option<u64> {
        let (_, rest) = last.split_once(&format!("\"{field}\":\""))?;
        rest.split('"').next()?.parse().ok()
    };
    let total = number("device_size").filter(|total| *total > 0)?;
    u8::try_from(number("device_bytes")?.saturating_mul(100) / total).ok()
}

fn read_disk() -> DiskEncryption {
    let status = fs::read_to_string(DISK_STATUS).unwrap_or_default();
    match status.split_whitespace().next() {
        Some("on") => DiskEncryption::On,
        Some("off") => DiskEncryption::Off,
        Some("encrypting") => DiskEncryption::Encrypting(
            fs::read_to_string(DISK_PROGRESS)
                .ok()
                .and_then(|progress| percent_done(&progress))
                .unwrap_or(0),
        ),
        _ => DiskEncryption::Plain,
    }
}

fn read_remote_login(system: Option<&Connection>) -> bool {
    let Some(system) = system else {
        return false;
    };
    let state: zbus::Result<String> = system
        .call_method(
            Some("org.freedesktop.systemd1"),
            "/org/freedesktop/systemd1",
            Some("org.freedesktop.systemd1.Manager"),
            "GetUnitFileState",
            &(REMOTE_LOGIN_UNIT,),
        )
        .and_then(|reply| reply.body().deserialize());
    state.is_ok_and(|state| state == "enabled")
}

pub fn follow_security_status(
    system: Option<&Connection>,
    updates: Updates,
) -> Option<FolderWatch> {
    let remote_login = read_remote_login(system);
    let read = move || SecurityStatus {
        disk: read_disk(),
        remote_login,
    };
    updates.send(Update::Security(read()));
    let watching = updates.clone();
    let watch = FolderWatch::spawn(Changes::Any, move |_| {
        watching.send(Update::Security(read()));
    })
    .ok()?;
    watch.watch(Path::new(DISK_DIRECTORY)).ok()?;
    Some(watch)
}

impl SettingsView {
    pub(crate) fn security_status_rows(&mut self) -> Widgets {
        let status = self.security;
        let disk_detail = match status.disk {
            DiskEncryption::Plain => L10N.tr("disk-encryption-plain"),
            DiskEncryption::Off => L10N.tr("disk-encryption-off"),
            DiskEncryption::Encrypting(percent) => L10N.tr_args(
                "disk-encryption-encrypting",
                [("percent", percent.to_string().into())],
            ),
            DiskEncryption::On => L10N.tr("disk-encryption-on"),
        };
        let remote_detail = if status.remote_login {
            L10N.tr("remote-login-on")
        } else {
            L10N.tr("remote-login-off")
        };
        vec![
            boxed(
                Row::new(Text::new(&L10N.tr("disk-encryption"), Tone::Primary), None)
                    .detail(Text::new(&disk_detail, Tone::Muted).caption()),
            ),
            boxed(
                Row::new(Text::new(&L10N.tr("remote-login"), Tone::Primary), None)
                    .detail(Text::new(&remote_detail, Tone::Muted).caption()),
            ),
            boxed(
                Row::new(Text::new(&L10N.tr("native-apps"), Tone::Primary), None)
                    .detail(Text::new(&L10N.tr("native-apps-reach"), Tone::Muted).caption()),
            ),
        ]
    }
}
