use jiff::Timestamp;
use jiff::tz::TimeZone;
use unbound_settings_panes::SettingId;
use unbound_toolkit::Spacing;
use unbound_toolkit::ui::{Axis, Button, SearchField, Stack, TextField, Widget};

use crate::L10N;
use crate::pages::{Widgets, boxed, muted_row, submit};
use crate::time::{MANUAL_FORMAT, zone_offset, zone_place};
use crate::view::{Anchor, Intent, Picker, SettingsView, intent};
use crate::widgets::{Row, Text, Tone};

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
        let field = TextField::new(&L10N.tr("manual-time-placeholder"), &text, move |text| {
            draft(text.to_owned())
        });
        self.anchors.push((Anchor::Setting(id, None), field.id()));
        vec![boxed(
            Row::new(
                Text::new(&L10N.tr("date-and-time"), Tone::Primary),
                Some(boxed(field)),
            )
            .detail(Text::new(&L10N.tr("manual-time-hint"), Tone::Muted).caption()),
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
                let button = Button::new(&L10N.tr("change-ellipsis"), move || change(()));
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
            &L10N.tr("search-zones"),
            &query,
            submit(&self.intents, Intent::PickQuery),
        );
        self.anchors.push((Anchor::Picker, field.id()));
        let mut cancel = intent(&self.intents, |()| Intent::Pick(None));
        let mut rows = vec![boxed(Row::new(
            Text::new(&L10N.tr("choose-time-zone"), Tone::Primary),
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
        if results.is_empty() {
            rows.push(muted_row(&L10N.tr(if query.trim().is_empty() {
                "zone-hint"
            } else {
                "no-zones"
            })));
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
