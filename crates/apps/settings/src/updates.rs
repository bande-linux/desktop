use bande_fs::format_size;
use bande_settings_panes::SettingId;
use bande_toolkit::ui::{Button, Label, ProgressBar, Spinner, Widget};

use crate::L10N;
use crate::pages::{Widgets, boxed};
use crate::privilege::Privilege;
use crate::software::{Package, SoftwareCommand, SoftwareState, Stage};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

const LISTED_UPDATES: usize = 12;

impl SettingsView {
    fn software_button(
        &mut self,
        title: &str,
        command: SoftwareCommand,
        primary: bool,
    ) -> Box<dyn Widget> {
        let refused = matches!(command, SoftwareCommand::Install(_))
            && self.refused.contains(&Privilege::Updates);
        let mut act = intent(&self.intents, Intent::Software);
        let mut button = Button::new(title, move || act(command.clone()));
        button.set_enabled(!refused);
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
                let check =
                    self.software_button(&L10N.tr("check-now"), SoftwareCommand::Check, false);
                vec![self.status_row(&L10N.tr("up-to-date"), Some(&system), Some(check))]
            }
            SoftwareState::Available(packages) => {
                let total: u64 = packages.iter().map(|package| package.size).sum();
                let detail = if total > 0 {
                    L10N.tr_args("download-size", [("size", format_size(total).into())])
                } else {
                    system
                };
                let ids = packages.iter().map(|package| package.id.clone()).collect();
                let refused = self.refused.contains(&Privilege::Updates);
                let detail = match refused {
                    true => L10N.tr("administrator-only"),
                    false => detail,
                };
                let install = self.software_button(
                    &L10N.tr("install-now"),
                    SoftwareCommand::Install(ids),
                    !refused,
                );
                let mut rows = vec![self.status_row(
                    &L10N.tr_args("updates-available", [("count", packages.len().into())]),
                    Some(&detail),
                    Some(install),
                )];
                rows.extend(Self::package_rows(&packages));
                rows
            }
            SoftwareState::Installing { percent } => {
                let label = L10N.tr("installing-updates");
                vec![self.status_row(
                    &label,
                    Some(&L10N.tr("installing-hint")),
                    Some(boxed(ProgressBar::new(
                        &label,
                        percent.map(|percent| percent as f32 / 100.0),
                    ))),
                )]
            }
            SoftwareState::Installed { restart: true } => {
                let restart =
                    self.software_button(&L10N.tr("restart-now"), SoftwareCommand::Restart, true);
                vec![self.status_row(
                    &L10N.tr("updates-installed"),
                    Some(&L10N.tr("restart-to-finish")),
                    Some(restart),
                )]
            }
            SoftwareState::Installed { restart: false } => {
                let check =
                    self.software_button(&L10N.tr("check-now"), SoftwareCommand::Check, false);
                vec![self.status_row(&L10N.tr("updates-installed"), Some(&system), Some(check))]
            }
            SoftwareState::Unavailable => vec![self.status_row(
                &L10N.tr("updates-unavailable"),
                Some(&L10N.tr("updates-unavailable-hint")),
                None,
            )],
            SoftwareState::Failed { during, reason } => {
                let title = match during {
                    Stage::Check => L10N.tr("updates-failed"),
                    Stage::Install => L10N.tr("install-failed"),
                };
                let retry =
                    self.software_button(&L10N.tr("try-again"), SoftwareCommand::Check, false);
                vec![self.status_row(&title, Some(&reason), Some(retry))]
            }
        }
    }
}
