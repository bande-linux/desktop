use crate::L10N;
use crate::battery::Charge;
use crate::pages::{Widgets, boxed};
use crate::view::SettingsView;
use crate::widgets::{LevelHistory, Row, Text, Tone};

fn duration(minutes: u32) -> String {
    let (hours, minutes) = (minutes / 60, minutes % 60);
    if hours == 0 {
        L10N.tr_args("duration-minutes", [("minutes", minutes.into())])
    } else {
        L10N.tr_args(
            "duration-hours-minutes",
            [("hours", hours.into()), ("minutes", minutes.into())],
        )
    }
}

pub fn charge_text(charge: Charge) -> String {
    match charge {
        Charge::Full => L10N.tr("battery-charged"),
        Charge::Charging {
            minutes: Some(minutes),
        } => L10N.tr_args("battery-until-full", [("time", duration(minutes).into())]),
        Charge::Charging { minutes: None } => L10N.tr("battery-charging"),
        Charge::Discharging {
            minutes: Some(minutes),
        } => L10N.tr_args("battery-remaining", [("time", duration(minutes).into())]),
        Charge::Discharging { minutes: None } => L10N.tr("battery-on-battery"),
    }
}

impl SettingsView {
    pub(crate) fn battery_rows(&mut self) -> Widgets {
        let Some(battery) = self.battery.clone() else {
            return Vec::new();
        };
        let percent = L10N.tr_args(
            "battery-percent",
            [("percent", battery.percent.round().into())],
        );
        let mut rows = vec![boxed(
            Row::new(Text::new(&percent, Tone::Primary), None)
                .detail(Text::new(&charge_text(battery.charge), Tone::Muted).caption()),
        )];
        if battery.history.len() > 1 {
            rows.push(boxed(LevelHistory::new(
                &L10N.tr("battery-history"),
                battery.history,
                [
                    L10N.tr("hours-ago-24"),
                    L10N.tr("hours-ago-12"),
                    L10N.tr("now"),
                ],
            )));
        }
        rows
    }
}
