use bande_l10n::tr;

use crate::battery::Charge;
use crate::pages::{Widgets, boxed};
use crate::view::SettingsView;
use crate::widgets::{LevelHistory, Row, Text, Tone};

fn duration(minutes: u32) -> String {
    let (hours, minutes) = (minutes / 60, minutes % 60);
    if hours == 0 {
        tr!("{minutes} min", minutes = minutes)
    } else {
        tr!("{hours} h {minutes} min", hours = hours, minutes = minutes)
    }
}

pub fn charge_text(charge: Charge) -> String {
    match charge {
        Charge::Full => tr!("Charged"),
        Charge::Charging {
            minutes: Some(minutes),
        } => tr!("Charging, {time} until full", time = duration(minutes)),
        Charge::Charging { minutes: None } => tr!("Charging"),
        Charge::Discharging {
            minutes: Some(minutes),
        } => tr!("On battery, {time} left", time = duration(minutes)),
        Charge::Discharging { minutes: None } => tr!("On battery"),
    }
}

impl SettingsView {
    pub(crate) fn battery_rows(&mut self) -> Widgets {
        let Some(battery) = self.battery.clone() else {
            return Vec::new();
        };
        let percent = tr!("{percent}%", percent = battery.percent.round());
        let mut rows = vec![boxed(
            Row::new(Text::new(&percent, Tone::Primary), None)
                .detail(Text::new(&charge_text(battery.charge), Tone::Muted).caption()),
        )];
        if battery.history.len() > 1 {
            rows.push(boxed(LevelHistory::new(
                &tr!("Battery level over the last 24 hours"),
                battery.history,
                [tr!("24 hours ago"), tr!("12 hours ago"), tr!("Now")],
            )));
        }
        rows
    }
}
