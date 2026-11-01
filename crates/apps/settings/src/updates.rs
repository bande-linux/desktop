use bande_fs::format_size;
use bande_packages::Class;
use bande_settings_panes::SettingId;
use bande_toolkit::ui::{Axis, Button, Label, ProgressBar, Spinner, Stack, Widget};
use bande_toolkit::{SessionEnd, Spacing};

use crate::L10N;
use crate::pages::{Widgets, boxed};
use crate::software::{Need, Package, Reboot, SoftwareCommand, SoftwareState, Stage};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

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
        let later = self.software_button(&L10N.tr("later"), Intent::Later(need), false);
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
                &L10N.tr("check-now"),
                Intent::Software(SoftwareCommand::Check),
                false,
            );
            rows.push(self.status_row(
                &L10N.tr_args("updates-were-installed", [("count", count.into())]),
                Some(system),
                Some(check),
            ));
        }
        if let Some(reboot) = reboot {
            let (title, detail) = match reboot {
                Reboot::Finish => (
                    L10N.tr("restart-to-finish"),
                    L10N.tr("restart-to-finish-hint"),
                ),
                Reboot::Install(count) => (
                    L10N.tr_args("restart-to-install", [("count", count.into())]),
                    L10N.tr("installs-on-restart"),
                ),
            };
            let buttons =
                self.later_buttons(&L10N.tr("restart-now"), SessionEnd::Restart, Need::Reboot);
            rows.push(self.status_row(&title, Some(&detail), Some(buttons)));
        }
        if logout {
            let buttons = self.later_buttons(&L10N.tr("log-out"), SessionEnd::LogOut, Need::LogOut);
            rows.push(self.status_row(
                &L10N.tr("log-out-to-finish"),
                Some(&L10N.tr("log-out-to-finish-hint")),
                Some(buttons),
            ));
        }
        if let Some((last, first)) = apps.split_last() {
            let listed = if first.is_empty() {
                last.clone()
            } else {
                L10N.tr_args(
                    "apps-and",
                    [
                        ("apps", first.join(", ").into()),
                        ("last", last.clone().into()),
                    ],
                )
            };
            rows.push(self.status_row(
                &L10N.tr_args("reopen-apps", [("apps", listed.into())]),
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
                boxed(
                    Row::new(Text::new(&package.name, Tone::Primary), size)
                        .detail(Text::new(&package.version, Tone::Muted).caption()),
                )
            })
            .collect();
        if packages.len() > LISTED_UPDATES {
            rows.push(boxed(Row::new(
                Text::new(
                    &L10N.tr_args(
                        "more-updates",
                        [("count", (packages.len() - LISTED_UPDATES).into())],
                    ),
                    Tone::Muted,
                ),
                None,
            )));
        }
        rows
    }

    pub(crate) fn software_rows(&mut self) -> Widgets {
        let system = self
            .about
            .as_ref()
            .map(|about| about.system.clone())
            .unwrap_or_default();
        match self.software.clone() {
            SoftwareState::Unknown | SoftwareState::Checking => vec![self.status_row(
                &L10N.tr("checking-updates"),
                None,
                Some(boxed(Spinner::new(&L10N.tr("checking-updates")))),
            )],
            SoftwareState::UpToDate => {
                let check = self.software_button(
                    &L10N.tr("check-now"),
                    Intent::Software(SoftwareCommand::Check),
                    false,
                );
                vec![self.status_row(&L10N.tr("up-to-date"), Some(&system), Some(check))]
            }
            SoftwareState::Available(packages) => {
                let total: u64 = packages.iter().map(|package| package.size).sum();
                let size = [("size", format_size(total).into())];
                let restarts = packages.iter().any(|package| package.class != Class::Live);
                let detail = match (total > 0, restarts) {
                    (true, true) => L10N.tr_args("download-size-restart", size),
                    (true, false) => L10N.tr_args("download-size", size),
                    (false, true) => L10N.tr("updates-need-restart"),
                    (false, false) => system,
                };
                let download = self.software_button(
                    &L10N.tr("update-now"),
                    Intent::Software(SoftwareCommand::Update(packages.clone())),
                    true,
                );
                let mut rows = vec![self.status_row(
                    &L10N.tr_args("updates-available", [("count", packages.len().into())]),
                    Some(&detail),
                    Some(download),
                )];
                rows.extend(Self::package_rows(&packages));
                rows
            }
            SoftwareState::Downloading { percent } => vec![self.progress_row(
                &L10N.tr("downloading-updates"),
                &L10N.tr("downloading-hint"),
                percent,
            )],
            SoftwareState::Installing { percent } => vec![self.progress_row(
                &L10N.tr("installing-updates"),
                &L10N.tr("installing-hint"),
                percent,
            )],
            SoftwareState::Prepared(count) => {
                let restart = self.software_button(
                    &L10N.tr("restart-now"),
                    Intent::EndSession(SessionEnd::Restart),
                    true,
                );
                vec![self.status_row(
                    &L10N.tr_args("restart-to-install", [("count", count.into())]),
                    Some(&L10N.tr("installs-on-restart")),
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
                    &L10N.tr("finish-installing"),
                    Intent::Software(SoftwareCommand::Repair),
                    true,
                );
                vec![self.status_row(
                    &L10N.tr("updates-interrupted"),
                    Some(&L10N.tr("updates-interrupted-hint")),
                    Some(finish),
                )]
            }
            SoftwareState::Unavailable => vec![self.status_row(
                &L10N.tr("updates-unavailable"),
                Some(&L10N.tr("updates-unavailable-hint")),
                None,
            )],
            SoftwareState::Failed { during, reason } => {
                let title = match during {
                    Stage::Check => L10N.tr("updates-failed"),
                    Stage::Download => L10N.tr("download-failed"),
                    Stage::Install => L10N.tr("install-failed"),
                };
                let retry = self.software_button(
                    &L10N.tr("try-again"),
                    Intent::Software(SoftwareCommand::Check),
                    false,
                );
                vec![self.status_row(&title, Some(&reason), Some(retry))]
            }
        }
    }
}
