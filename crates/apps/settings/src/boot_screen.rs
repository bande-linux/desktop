use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Sender};

use bande_l10n::tr;
use bande_packages::Client;
use bande_toolkit::ui::Toggle;
use zbus::blocking::Connection;

use crate::follow::{Update, Updates, spawn};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const HELPER: &str = "/usr/libexec/bande-boot-screen-helper";
const PKEXEC: &str = "/usr/bin/pkexec";
const THEME_PACKAGE: &str = "bande-plymouth-theme";
const THEME: &str = "/usr/share/plymouth/themes/bande/bande.plymouth";
/// GRUB's configuration file, shipped by `bande-plymouth-theme`, that adds `splash` to the
/// kernel command line.
pub const SPLASH_DROP_IN: &str = "/etc/default/grub.d/bande-splash.cfg";
const SPLASH: &str = r#"case " $GRUB_CMDLINE_LINUX_DEFAULT " in *" splash "*) ;; *) GRUB_CMDLINE_LINUX_DEFAULT="$GRUB_CMDLINE_LINUX_DEFAULT splash" ;; esac"#;
const GRUB_CONFIG: &str = "/boot/grub/grub.cfg";
const UPDATE_GRUB: &str = "/usr/sbin/update-grub";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootScreenState {
    pub grub: bool,
    pub shown: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootScreenCommand {
    Read,
    Show(bool),
}

fn adds_splash(line: &str) -> bool {
    let line = line.trim_start();
    !line.starts_with('#') && line.contains("splash")
}

/// Returns the text of [`SPLASH_DROP_IN`] with its `splash` line active or commented out,
/// adding the line when it is to be active and none is.
pub fn switch_splash(text: &str, on: bool) -> String {
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| match on {
            true if line.trim_start_matches(['#', ' ', '\t']) == SPLASH => SPLASH.to_owned(),
            false if adds_splash(line) => format!("#{line}"),
            _ => line.to_owned(),
        })
        .collect();
    if on && !lines.iter().any(|line| adds_splash(line)) {
        lines.push(SPLASH.to_owned());
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

fn splash_added() -> bool {
    match fs::read_to_string(SPLASH_DROP_IN) {
        Ok(text) => text.lines().any(adds_splash),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!("cannot read {SPLASH_DROP_IN}: {error}");
            false
        }
    }
}

fn read_state() -> BootScreenState {
    BootScreenState {
        grub: Path::new(UPDATE_GRUB).exists() && Path::new(GRUB_CONFIG).exists(),
        shown: Path::new(THEME).exists() && splash_added(),
        failed: false,
    }
}

fn helper(on: bool) -> bool {
    Command::new(PKEXEC)
        .arg(HELPER)
        .arg(if on { "on" } else { "off" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .inspect_err(|error| tracing::warn!("cannot run the boot screen helper: {error}"))
        .is_ok_and(|status| status.success())
}

fn install_theme(system: Option<&Connection>) -> bool {
    Path::new(THEME).exists()
        || system.is_some_and(|system| {
            Client::new(system)
                .install_missing(&[THEME_PACKAGE])
                .inspect_err(|error| tracing::warn!("cannot install {THEME_PACKAGE}: {error}"))
                .is_ok()
        })
}

fn run(command: BootScreenCommand, system: Option<&Connection>) -> bool {
    match command {
        BootScreenCommand::Read => true,
        BootScreenCommand::Show(true) => install_theme(system) && helper(true),
        BootScreenCommand::Show(false) => helper(false),
    }
}

pub fn follow_boot_screen(
    system: Option<Connection>,
    updates: Updates,
) -> Sender<BootScreenCommand> {
    let (sender, receiver) = mpsc::channel();
    spawn("boot-screen", move || {
        for command in receiver {
            let worked = run(command, system.as_ref());
            let state = BootScreenState {
                failed: !worked,
                ..read_state()
            };
            if !updates.send(Update::BootScreen(state)) {
                return;
            }
        }
    });
    sender
}

impl SettingsView {
    pub(crate) fn boot_screen_rows(&mut self) -> Widgets {
        let Some(state) = self.boot_screen.clone() else {
            return Vec::new();
        };
        if !state.grub {
            return vec![muted_row(&tr!(
                "This computer doesn’t start with GRUB, so Settings can’t turn on the boot screen. Install bande-plymouth-theme and add “splash” to the kernel command line of its boot manager."
            ))];
        }
        let mut rows = Vec::new();
        if state.failed {
            rows.push(muted_row(&tr!("That change didn’t go through.")));
        }
        let mut change = intent(&self.intents, Intent::BootScreen);
        let toggle = Toggle::new(&tr!("Show a boot screen"), state.shown, move |on| {
            change(BootScreenCommand::Show(on));
        });
        let detail = match self.changing_boot_screen {
            true => tr!("Changing…"),
            false => tr!(
                "The Bande mark instead of the computer’s messages while it starts and shuts down. It takes effect at the next start."
            ),
        };
        let row = Row::new(
            Text::new(&tr!("Show a boot screen"), Tone::Primary),
            Some(boxed(toggle)),
        )
        .detail(Text::new(&detail, Tone::Muted).caption());
        rows.push(self.item_row("boot-screen".to_owned(), row));
        rows
    }
}
