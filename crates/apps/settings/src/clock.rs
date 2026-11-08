use bande_l10n::tr;

use crate::pages::{Widgets, boxed, muted_row, submit};
use crate::time::{MANUAL_FORMAT, zone_offset, zone_place};
use crate::view::{Anchor, Intent, Picker, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};
use bande_settings_panes::SettingId;
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Button, SearchField, Stack, TextField, Widget};
use jiff::Timestamp;
use jiff::tz::TimeZone;

fn zone_detail(region: &str, zone: &str, now: Timestamp) -> String {
    [Some(region.to_owned()), zone_offset(zone, now)]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

impl SettingsView {
    pub(crate) fn clock_rows(&mut self) -> Widgets {
        let id = SettingId::DateAndTime;
        let Some(time) = self
            .time
            .clone()
            .filter(|time| !time.automatic && self.allowed(id))
        else {
            return Vec::new();
        };
        let zone = TimeZone::get(&time.zone).unwrap_or(TimeZone::UTC);
        let now = Timestamp::now().to_zoned(zone);
        let text = self
            .drafts
            .get(&id)
            .cloned()
            .unwrap_or_else(|| now.strftime(MANUAL_FORMAT).to_string());
        let mut draft = intent(&self.intents, move |text: String| Intent::Draft(id, text));
        let field = TextField::new(&tr!("YYYY-MM-DD HH:MM"), &text, move |text| {
            draft(text.to_owned())
        })
        .without_text_checking();
        self.anchors.push((Anchor::Setting(id, None), field.id()));
        vec![boxed(
            Row::new(
                Text::new(&tr!("Date and time"), Tone::Primary),
                Some(boxed(field)),
            )
            .detail(Text::new(&tr!("Press Return to set the clock"), Tone::Muted).caption()),
        )]
    }

    pub(crate) fn zone_rows(&mut self) -> Widgets {
        let Some(time) = self.time.clone() else {
            return Vec::new();
        };
        let now = Timestamp::now();
        let Some((_, query)) = self
            .picking
            .clone()
            .filter(|(picker, _)| *picker == Picker::Zone)
        else {
            let (city, region) = zone_place(&time.zone);
            let button = self.allowed(SettingId::TimeZone).then(|| {
                let mut change = intent(&self.intents, |()| Intent::Pick(Some(Picker::Zone)));
                let button = Button::new(&tr!("Change…"), move || change(()));
                self.anchors
                    .push((Anchor::Setting(SettingId::TimeZone, None), button.id()));
                boxed(button)
            });
            return vec![boxed(
                Row::new(Text::new(&city, Tone::Primary), button).detail(
                    Text::new(&zone_detail(&region, &time.zone, now), Tone::Muted).caption(),
                ),
            )];
        };
        let field = SearchField::new(
            &tr!("Search cities"),
            &query,
            submit(&self.intents, Intent::PickQuery),
        );
        self.anchors.push((Anchor::Picker, field.id()));
        let mut cancel = intent(&self.intents, |()| Intent::Pick(None));
        let mut rows = vec![boxed(Row::new(
            Text::new(&tr!("Choose a time zone"), Tone::Primary),
            Some(boxed(Stack::new(
                Axis::Horizontal,
                Spacing::S as f32,
                0.0,
                vec![
                    boxed(field),
                    boxed(Button::new(&tr!("Cancel"), move || cancel(())).cancel()),
                ],
            ))),
        ))];
        let results = self.pick_results();
        if results.is_empty() {
            rows.push(muted_row(&if query.trim().is_empty() {
                tr!("Type the name of a city near you")
            } else {
                tr!("No time zone matches")
            }));
        }
        for zone in results {
            let (city, region) = zone_place(&zone);
            let mut choose = intent(&self.intents, Intent::Picked);
            let chosen = zone.clone();
            let row = Row::new(Text::new(&city, Tone::Primary), None)
                .detail(Text::new(&zone_detail(&region, &zone, now), Tone::Muted).caption())
                .on_activate(move || choose(chosen.clone()));
            rows.push(self.item_row(format!("zone:{zone}"), row));
        }
        rows
    }
}
