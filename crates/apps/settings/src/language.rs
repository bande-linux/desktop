use bande_datetime::{DateFormats, posix_locale};
use bande_settings_panes::SettingId;
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, SearchField, Stack, Widget};

use crate::locale::{AvailableLocale, LocaleState};
use crate::pages::{Widgets, boxed, muted_row, submit};
use crate::view::{Anchor, Intent, Picker, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use crate::{L10N, TRANSLATIONS};

pub const SAME_AS_LANGUAGE: &str = "";

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

pub fn locale_matches(available: &[AvailableLocale], query: &str, limit: usize) -> Vec<String> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let Some(first) = words.first() else {
        return Vec::new();
    };
    let query = words.join(" ");
    let mut ranked: Vec<(u8, &AvailableLocale)> = available
        .iter()
        .filter_map(|locale| {
            let name = locale.name.to_lowercase();
            let code = locale.code.to_lowercase();
            if !words
                .iter()
                .all(|word| name.contains(word.as_str()) || code.starts_with(word.as_str()))
            {
                return None;
            }
            let rank = if name.starts_with(&query) || code.starts_with(&query) {
                0
            } else if name
                .split(|character: char| !character.is_alphanumeric())
                .any(|word| word.starts_with(first.as_str()))
            {
                1
            } else {
                2
            };
            Some((rank, locale))
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| *rank);
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, locale)| locale.code.clone())
        .collect()
}

pub fn translated(language: &str) -> bool {
    let tongue = language.split(['_', '.', '@']).next().unwrap_or(language);
    tongue == "C"
        || tongue == "POSIX"
        || TRANSLATIONS
            .iter()
            .any(|(tag, _)| tag.split('-').next() == Some(tongue))
}

fn locale_label(locale: &LocaleState, code: &str) -> String {
    locale
        .available
        .iter()
        .find(|available| same_locale(&available.code, code))
        .map_or_else(|| code.to_owned(), |available| available.name.clone())
}

impl SettingsView {
    fn locale_row(&mut self, id: SettingId, label: &str, shown: &str, picker: Picker) -> Widgets {
        let button = self.allowed(id).then(|| {
            let mut change = intent(&self.intents, move |()| Intent::Pick(Some(picker)));
            let button = Button::new(&L10N.tr("change-ellipsis"), move || change(()));
            self.anchors.push((Anchor::Setting(id, None), button.id()));
            boxed(button)
        });
        vec![boxed(
            Row::new(Text::new(label, Tone::Primary), button)
                .detail(Text::new(shown, Tone::Muted).caption()),
        )]
    }

    fn locale_search(&mut self, locale: &LocaleState, title: &str, query: &str) -> Widgets {
        let field = SearchField::new(
            &L10N.tr("search-languages"),
            query,
            submit(&self.intents, Intent::PickQuery),
        );
        self.anchors.push((Anchor::Picker, field.id()));
        let mut cancel = intent(&self.intents, |()| Intent::Pick(None));
        let mut rows = vec![boxed(Row::new(
            Text::new(title, Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&L10N.tr("cancel"), move || cancel(())).cancel()),
                ],
            ))),
        ))];
        let results = self.pick_results();
        for code in &results {
            let name = if code == SAME_AS_LANGUAGE {
                L10N.tr("same-as-language")
            } else {
                locale_label(locale, code)
            };
            let mut choose = intent(&self.intents, Intent::Picked);
            let chosen = code.clone();
            let row = Row::new(Text::new(&name, Tone::Primary), None)
                .detail(Text::new(code, Tone::Muted).caption())
                .on_activate(move || choose(chosen.clone()));
            rows.push(self.item_row(format!("locale:{code}"), row));
        }
        if query.trim().is_empty() {
            rows.push(muted_row(&L10N.tr("language-hint")));
        } else if results.is_empty() {
            rows.push(muted_row(&L10N.tr("no-languages")));
        }
        rows
    }

    fn picking_locale(&self, wanted: Picker) -> Option<String> {
        self.picking
            .clone()
            .filter(|(picker, _)| *picker == wanted)
            .map(|(_, query)| query)
    }

    pub(crate) fn language_rows(&mut self) -> Widgets {
        let Some(locale) = self.locale.clone() else {
            return Vec::new();
        };
        if locale.available.is_empty() {
            return vec![muted_row(&L10N.tr("no-locales"))];
        }
        let mut rows = match self.picking_locale(Picker::Language) {
            Some(query) => self.locale_search(&locale, &L10N.tr("choose-language"), &query),
            None => {
                let name = locale_label(&locale, &locale.language);
                let shown = if locale.pending {
                    L10N.tr_args("locale-setting-up", [("name", name.into())])
                } else {
                    name
                };
                self.locale_row(
                    SettingId::Language,
                    &L10N.tr("language"),
                    &shown,
                    Picker::Language,
                )
            }
        };
        if !translated(&locale.language) {
            rows.push(boxed(Row::new(
                Text::new(&L10N.tr("locale-own-apps-english"), Tone::Muted).caption(),
                None,
            )));
        }
        rows.push(boxed(Row::new(
            Text::new(&L10N.tr("locale-next-login"), Tone::Muted).caption(),
            None,
        )));
        rows
    }

    pub(crate) fn region_rows(&mut self) -> Widgets {
        let Some(locale) = self.locale.clone() else {
            return Vec::new();
        };
        if locale.available.is_empty() {
            return Vec::new();
        }
        let mut rows = match self.picking_locale(Picker::Region) {
            Some(query) => self.locale_search(&locale, &L10N.tr("choose-region"), &query),
            None => {
                let shown = locale.region.as_deref().map_or_else(
                    || L10N.tr("same-as-language"),
                    |region| locale_label(&locale, region),
                );
                self.locale_row(
                    SettingId::Region,
                    &L10N.tr("region"),
                    &shown,
                    Picker::Region,
                )
            }
        };
        let formats = DateFormats::for_locale(&posix_locale(
            locale.region.as_deref().unwrap_or(&locale.language),
        ));
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
