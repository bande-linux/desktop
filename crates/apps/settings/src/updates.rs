use bande_l10n::{tr, trn};

use crate::pages::{Widgets, boxed};
use crate::software::{Need, Package, Reboot, SoftwareCommand, SoftwareState, Stage};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use bande_fs::format_size;
use bande_packages::{Applies, Class};
use bande_settings_panes::SettingId;
use bande_toolkit::ui::{Axis, Button, Label, ProgressBar, Spinner, Stack, Widget};
use bande_toolkit::{SessionEnd, Spacing};
use jiff::Timestamp;
use jiff::tz::TimeZone;

const LISTED_UPDATES: usize = 12;

impl SettingsView {
    fn software_button(&mut self, title: &str, action: Intent, primary: bool) -> Box<dyn Widget> {
        let mut act = intent(&self.intents, move |()| action.clone());
        let button = Button::new(title, move || act(()));
        let button = if primary { button.primary() } else { button };
        self.anchors.push((
            Anchor::Setting(SettingId::SoftwareUpdates, None),
            button.id(),
        ));
        boxed(button)
    }

    fn status_row(
        &mut self,
        title: &str,
        detail: Option<&str>,
        control: Option<Box<dyn Widget>>,
    ) -> Box<dyn Widget> {
        let mut row = Row::new(Text::new(title, Tone::Primary), control);
        if let Some(detail) = detail {
            row = row.detail(Text::new(detail, Tone::Muted).caption());
        }
        boxed(row)
    }

    fn later_buttons(&mut self, title: &str, ending: SessionEnd, need: Need) -> Box<dyn Widget> {
        let later = self.software_button(&tr!("Later"), Intent::Later(need), false);
        let now = self.software_button(title, Intent::EndSession(ending), true);
        boxed(Stack::new(
            Axis::Horizontal,
            Spacing::S as f32,
            0.0,
            vec![later, now],
        ))
    }

    fn progress_row(&mut self, label: &str, hint: &str, percent: Option<u32>) -> Box<dyn Widget> {
        let bar = ProgressBar::new(label, percent.map(|percent| percent as f32 / 100.0));
        self.status_row(label, Some(hint), Some(boxed(bar)))
    }

    fn installed_rows(
        &mut self,
        count: usize,
        reboot: Option<Reboot>,
        logout: bool,
        apps: &[String],
        failed: Option<&str>,
        system: &str,
    ) -> Widgets {
        let mut rows = Vec::new();
        if let Some(failed) = failed {
            let retry = self.software_button(
                &tr!("Try Again"),
                Intent::Software(SoftwareCommand::Check),
                false,
            );
            rows.push(self.status_row(
                &tr!("Some updates didn't install"),
                Some(failed),
                Some(retry),
            ));
        }
        if count > 0 {
            let check = self.software_button(
                &tr!("Check Now"),
                Intent::Software(SoftwareCommand::Check),
                false,
            );
            rows.push(self.status_row(
                &trn!(
                    "1 update was installed",
                    "{count} updates were installed",
                    count = count
                ),
                Some(system),
                Some(check),
            ));
        }
        if let Some(reboot) = reboot {
            let (title, detail, action, ending) = match reboot {
                Reboot::Finish => (
                    tr!("Restart to finish updating"),
                    tr!("Some updates take effect after a restart"),
                    tr!("Restart Now"),
                    SessionEnd::Restart,
                ),
                Reboot::Install { count, ending } => (
                    ending.title(count),
                    ending.hint(),
                    ending.action(),
                    ending.session_end(),
                ),
            };
            let buttons = self.later_buttons(&action, ending, Need::Reboot);
            rows.push(self.status_row(&title, Some(&detail), Some(buttons)));
        }
        if logout {
            let buttons = self.later_buttons(&tr!("Log Out"), SessionEnd::LogOut, Need::LogOut);
            rows.push(self.status_row(
                &tr!("Log out to finish updating the desktop"),
                Some(&tr!(
                    "The desktop keeps running its old version until you log out"
                )),
                Some(buttons),
            ));
        }
        if let Some((last, first)) = apps.split_last() {
            let listed = if first.is_empty() {
                last.clone()
            } else {
                tr!(
                    "{apps} and {last}",
                    apps = first.join(", "),
                    last = last.clone()
                )
            };
            rows.push(self.status_row(
                &tr!(
                    "Quit and reopen {apps} to use the new versions",
                    apps = listed
                ),
                None,
                None,
            ));
        }
        rows
    }

    fn firmware_detail(package: &Package, applies: Applies) -> String {
        let versions = match package.from.is_empty() {
            true => tr!("Firmware {to}", to = package.version.clone()),
            false => tr!(
                "Firmware {from} → {to}",
                from = package.from.clone(),
                to = package.version.clone()
            ),
        };
        let versions = match applies {
            Applies::AtRestart => tr!("{versions} · Installs during restart", versions = versions),
            Applies::AtShutdown => {
                tr!("{versions} · Installs while shut down", versions = versions)
            }
            Applies::Now => versions,
        };
        match package.summary.is_empty() {
            true => versions,
            false => format!("{versions}\n{}", package.summary),
        }
    }

    fn package_rows(packages: &[Package]) -> Widgets {
        let mut rows: Widgets = packages
            .iter()
            .take(LISTED_UPDATES)
            .map(|package| {
                let size =
                    (package.size > 0).then(|| boxed(Label::new(&format_size(package.size))));
                let version = match package.class {
                    Class::Flatpak => tr!("{version} · Flatpak", version = package.version.clone()),
                    Class::Firmware(applies) => Self::firmware_detail(package, applies),
                    _ => package.version.clone(),
                };
                boxed(
                    Row::new(Text::new(&package.name, Tone::Primary), size)
                        .detail(Text::new(&version, Tone::Muted).caption()),
                )
            })
            .collect();
        if packages.len() > LISTED_UPDATES {
            rows.push(boxed(Row::new(
                Text::new(
                    &tr!(
                        "and {count} more",
                        count = (packages.len() - LISTED_UPDATES)
                    ),
                    Tone::Muted,
                ),
                None,
            )));
        }
        rows
    }

    fn last_check_row(&mut self) -> Option<Box<dyn Widget>> {
        let checked = Timestamp::from_second(self.software_checked?).ok()?;
        let date = self
            .date_formats()
            .long_date(&checked.to_zoned(TimeZone::system()));
        Some(self.status_row(&tr!("Last checked {date}", date = date), None, None))
    }

    fn firmware_offer_row(&mut self) -> Option<Box<dyn Widget>> {
        let offered = self.firmware_missing
            && !matches!(
                self.software,
                SoftwareState::Unavailable
                    | SoftwareState::Downloading { .. }
                    | SoftwareState::Installing { .. }
            );
        if !offered {
            return None;
        }
        let install = self.software_button(
            &tr!("Install…"),
            Intent::Software(SoftwareCommand::InstallFirmwareSupport),
            false,
        );
        Some(self.status_row(
            &tr!("Firmware updates for this computer and its devices"),
            Some(&tr!(
                "Installs fwupd, which also checks for new firmware once a day by itself."
            )),
            Some(install),
        ))
    }

    pub(crate) fn software_rows(&mut self) -> Widgets {
        let mut rows = self.update_rows();
        rows.extend(self.firmware_offer_row());
        rows.extend(self.last_check_row());
        rows
    }

    fn update_rows(&mut self) -> Widgets {
        let system = self
            .about
            .as_ref()
            .map(|about| about.system.clone())
            .unwrap_or_default();
        match self.software.clone() {
            SoftwareState::Unknown | SoftwareState::Checking => vec![self.status_row(
                &tr!("Checking for updates…"),
                None,
                Some(boxed(Spinner::new(&tr!("Checking for updates…")))),
            )],
            SoftwareState::UpToDate => {
                let check = self.software_button(
                    &tr!("Check Now"),
                    Intent::Software(SoftwareCommand::Check),
                    false,
                );
                vec![self.status_row(
                    &tr!("Your computer is up to date"),
                    Some(&system),
                    Some(check),
                )]
            }
            SoftwareState::Available(packages) => {
                let total: u64 = packages.iter().map(|package| package.size).sum();
                let size = format_size(total);
                let restarts = packages.iter().any(Package::needs_restart);
                let detail = match (total > 0, restarts) {
                    (true, true) => tr!(
                        "{size} to download. Some updates need a restart.",
                        size = size
                    ),
                    (true, false) => tr!("{size} to download", size = size),
                    (false, true) => tr!("Some updates need a restart"),
                    (false, false) => system,
                };
                let download = self.software_button(
                    &tr!("Update Now"),
                    Intent::Software(SoftwareCommand::Update(packages.clone())),
                    true,
                );
                let mut rows = vec![self.status_row(
                    &trn!(
                        "1 update is available",
                        "{count} updates are available",
                        count = packages.len()
                    ),
                    Some(&detail),
                    Some(download),
                )];
                rows.extend(Self::package_rows(&packages));
                rows
            }
            SoftwareState::Downloading { percent } => vec![self.progress_row(
                &tr!("Downloading updates…"),
                &tr!("You can keep working while updates download"),
                percent,
            )],
            SoftwareState::Installing { percent } => vec![self.progress_row(
                &tr!("Installing updates…"),
                &tr!("You can keep working while updates install"),
                percent,
            )],
            SoftwareState::Prepared { count, ending } => {
                let restart = self.software_button(
                    &ending.action(),
                    Intent::EndSession(ending.session_end()),
                    true,
                );
                vec![self.status_row(&ending.title(count), Some(&ending.hint()), Some(restart))]
            }
            SoftwareState::Installed {
                count,
                reboot,
                logout,
                apps,
                failed,
            } => self.installed_rows(count, reboot, logout, &apps, failed.as_deref(), &system),
            SoftwareState::Interrupted => {
                let finish = self.software_button(
                    &tr!("Finish Installing"),
                    Intent::Software(SoftwareCommand::Repair),
                    true,
                );
                vec![self.status_row(
                    &tr!("Updates didn't finish installing"),
                    Some(&tr!(
                        "Finish installing them before you install anything else"
                    )),
                    Some(finish),
                )]
            }
            SoftwareState::Unavailable => vec![self.status_row(
                &tr!("Updates aren't available on this computer"),
                Some(&tr!("Install the packagekit package")),
                None,
            )],
            SoftwareState::Failed { during, reason } => {
                let title = match during {
                    Stage::Check => tr!("Could not check for updates"),
                    Stage::Download => tr!("Could not download updates"),
                    Stage::Install => tr!("Could not install updates"),
                };
                let retry = self.software_button(
                    &tr!("Try Again"),
                    Intent::Software(SoftwareCommand::Check),
                    false,
                );
                vec![self.status_row(&title, Some(&reason), Some(retry))]
            }
        }
    }
}
