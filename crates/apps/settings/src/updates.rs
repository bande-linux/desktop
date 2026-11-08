use bande_l10n::{tr, trn};

use crate::pages::{Widgets, boxed};
use crate::software::versions::{self, Condition, Version};
use crate::software::{Need, Package, Reboot, SoftwareCommand, SoftwareState, Stage, restart_hint};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use bande_datetime::DateFormats;
use bande_fs::format_size;
use bande_packages::Class;
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
        system: &str,
    ) -> Widgets {
        let mut rows = Vec::new();
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
            let (title, detail) = match reboot {
                Reboot::Finish => (
                    tr!("Restart to finish updating"),
                    tr!("Some updates take effect after a restart"),
                ),
                Reboot::Install(count) => (
                    trn!(
                        "Restart to install 1 update",
                        "Restart to install {count} updates",
                        count = count
                    ),
                    restart_hint(),
                ),
            };
            let buttons =
                self.later_buttons(&tr!("Restart Now"), SessionEnd::Restart, Need::Reboot);
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

    fn package_rows(packages: &[Package]) -> Widgets {
        let mut rows: Widgets = packages
            .iter()
            .take(LISTED_UPDATES)
            .map(|package| {
                let size =
                    (package.size > 0).then(|| boxed(Label::new(&format_size(package.size))));
                let version = match package.class {
                    Class::Flatpak => tr!("{version} · Flatpak", version = package.version.clone()),
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

    fn version_row(&mut self, version: &Version, dates: &DateFormats) -> Box<dyn Widget> {
        let title = Timestamp::from_second(version.created)
            .map(|at| dates.long_date(&at.to_zoned(TimeZone::system())))
            .unwrap_or_default();
        let detail = match version.condition {
            Condition::Failed => tr!("This version couldn't start"),
            _ if version.changed > 0 => trn!(
                "1 update: {names}",
                "{count} updates: {names}",
                count = version.changed,
                names = version.names.clone()
            ),
            _ => tr!("Copy kept before a change"),
        };
        let control = match version.running {
            true => boxed(Label::new(&tr!("Running now"))),
            false => self.software_button(
                &tr!("Restore"),
                Intent::Software(SoftwareCommand::Restore(version.id)),
                false,
            ),
        };
        self.status_row(&title, Some(&detail), Some(control))
    }

    fn version_rows(&mut self) -> Widgets {
        let found = versions::read();
        if found.len() < 2 {
            return Vec::new();
        }
        let dates = DateFormats::from_env();
        let mut rows = vec![self.status_row(
            &tr!("Previous Versions"),
            Some(&tr!("Bring back an earlier system. Your files, accounts, passwords and Wi-Fi networks stay as they are now")),
            None,
        )];
        for version in found
            .iter()
            .filter(|version| version.condition != Condition::Pending)
        {
            rows.push(self.version_row(version, &dates));
        }
        rows
    }

    fn last_check_row(&mut self) -> Option<Box<dyn Widget>> {
        let checked = Timestamp::from_second(self.software_checked?).ok()?;
        let date = DateFormats::from_env().long_date(&checked.to_zoned(TimeZone::system()));
        Some(self.status_row(&tr!("Last checked {date}", date = date), None, None))
    }

    pub(crate) fn software_rows(&mut self) -> Widgets {
        let mut rows = self.update_rows();
        rows.extend(self.last_check_row());
        rows.extend(self.version_rows());
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
            SoftwareState::Prepared(count) => {
                let restart = self.software_button(
                    &tr!("Restart Now"),
                    Intent::EndSession(SessionEnd::Restart),
                    true,
                );
                vec![self.status_row(
                    &trn!(
                        "Restart to install 1 update",
                        "Restart to install {count} updates",
                        count = count
                    ),
                    Some(&restart_hint()),
                    Some(restart),
                )]
            }
            SoftwareState::Installed {
                count,
                reboot,
                logout,
                apps,
            } => self.installed_rows(count, reboot, logout, &apps, &system),
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
