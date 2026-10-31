use unbound_datetime::{DateFormats, posix_locale};
use unbound_settings_panes::SettingId;
use unbound_toolkit::ui::{Label, PopUpButton, Widget};

use crate::L10N;
use crate::locale::{InstalledLocale, LocaleCommand};
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Anchor, Intent, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

fn same_locale(a: &str, b: &str) -> bool {
    let key = |code: &str| code.to_lowercase().replace(['-', '_'], "");
    key(a) == key(b)
}

fn sample() -> Option<jiff::Zoned> {
    jiff::civil::date(2026, 3, 14)
        .at(13, 45, 0, 0)
        .to_zoned(jiff::tz::TimeZone::UTC)
        .ok()
}

impl SettingsView {
    fn locale_menu(
        &mut self,
        id: SettingId,
        label: &str,
        choices: Vec<(String, Option<String>)>,
        current: Option<&str>,
        command: fn(Option<String>) -> Option<LocaleCommand>,
    ) -> Box<dyn Widget> {
        let labels: Vec<String> = choices.iter().map(|(name, _)| name.clone()).collect();
        let selected = choices
            .iter()
            .position(|(_, code)| match (code, current) {
                (Some(code), Some(current)) => same_locale(code, current),
                (None, None) => true,
                _ => false,
            })
            .unwrap_or(0);
        if !self.allowed(id) {
            let shown = labels.get(selected).cloned().unwrap_or_default();
            return boxed(Row::new(
                Text::new(label, Tone::Primary),
                Some(boxed(Label::new(&shown))),
            ));
        }
        let mut send = intent(&self.intents, Intent::Locale);
        let menu = PopUpButton::new(label, &labels, selected, move |index| {
            if let Some(command) = choices
                .get(index)
                .and_then(|(_, code)| command(code.clone()))
            {
                send(command);
            }
        });
        self.anchors.push((Anchor::Setting(id, None), menu.id()));
        boxed(Row::new(Text::new(label, Tone::Primary), Some(boxed(menu))))
    }

    fn installed_choices(installed: &[InstalledLocale]) -> Vec<(String, Option<String>)> {
        installed
            .iter()
            .map(|locale| (locale.name.clone(), Some(locale.code.clone())))
            .collect()
    }

    pub(crate) fn language_rows(&mut self) -> Widgets {
        let Some(locale) = self.locale.clone() else {
            return Vec::new();
        };
        if locale.installed.is_empty() {
            return vec![muted_row(&L10N.tr("no-locales"))];
        }
        let choices = Self::installed_choices(&locale.installed);
        vec![
            self.locale_menu(
                SettingId::Language,
                &L10N.tr("language"),
                choices,
                Some(&locale.language),
                |code| code.map(LocaleCommand::Language),
            ),
            boxed(Row::new(
                Text::new(&L10N.tr("locale-next-login"), Tone::Muted).caption(),
                None,
            )),
        ]
    }

    pub(crate) fn region_rows(&mut self) -> Widgets {
        let Some(locale) = self.locale.clone() else {
            return Vec::new();
        };
        if locale.installed.is_empty() {
            return Vec::new();
        }
        let mut choices = vec![(L10N.tr("same-as-language"), None)];
        choices.extend(Self::installed_choices(&locale.installed));
        let region = locale.region.clone();
        let mut rows = vec![self.locale_menu(
            SettingId::Region,
            &L10N.tr("region"),
            choices,
            region.as_deref(),
            |code| Some(LocaleCommand::Region(code)),
        )];
        let formats =
            DateFormats::for_locale(&posix_locale(region.as_deref().unwrap_or(&locale.language)));
        if let Some(sample) = sample() {
            let preview = format!(
                "{} · {}",
                formats.long_date(&sample),
                formats.clock(&sample)
            );
            rows.push(boxed(
                Row::new(Text::new(&L10N.tr("formats-preview"), Tone::Primary), None)
                    .detail(Text::new(&preview, Tone::Muted).caption()),
            ));
        }
        rows
    }
}
