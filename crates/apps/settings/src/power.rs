use bande_l10n::tr;
use bande_toolkit::ui::Toggle;

use crate::battery::{BatteryCommand, Charge, ChargeLimit, Profiles};
use crate::general::info_row;
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{LevelHistory, Row, Text, Tone};

const HEALTHY_CAPACITY: f64 = 80.0;

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
        let Some(battery) = self.power.as_ref().and_then(|power| power.battery.clone()) else {
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
        if let Some(capacity) = battery.capacity {
            let health = match capacity >= HEALTHY_CAPACITY {
                true => tr!("Normal"),
                false => tr!("Service recommended"),
            };
            rows.push(boxed(
                Row::new(
                    Text::new(&tr!("Battery health"), Tone::Primary),
                    Some(boxed(bande_toolkit::ui::Label::new(&health))),
                )
                .detail(
                    Text::new(
                        &tr!("Maximum capacity {percent}%", percent = capacity.round()),
                        Tone::Muted,
                    )
                    .caption(),
                ),
            ));
        }
        if let Some(limit) = battery.limit {
            rows.push(self.charge_limit_row(limit));
        }
        rows
    }

    fn charge_limit_row(&mut self, limit: ChargeLimit) -> Box<dyn bande_toolkit::ui::Widget> {
        let label = match limit.end {
            Some(end) => tr!("Limit charging to {percent}%", percent = end),
            None => tr!("Limit charging"),
        };
        let set = intent(&self.intents, |on| {
            Intent::Battery(BatteryCommand::ChargeLimit(on))
        });
        boxed(
            Row::new(
                Text::new(&label, Tone::Primary),
                Some(boxed(Toggle::new(&label, limit.enabled, set))),
            )
            .detail(
                Text::new(
                    &tr!("Stops charging before full to slow the battery's ageing."),
                    Tone::Muted,
                )
                .caption(),
            ),
        )
    }

    pub(crate) fn power_status_rows(&mut self) -> Widgets {
        let Some(power) = self.power.clone() else {
            return vec![muted_row(&tr!(
                "Power information isn't available because the power service isn't running."
            ))];
        };
        let mut rows = Vec::new();
        if power.battery.is_none() {
            rows.push(info_row(&tr!("Power source"), &tr!("Power adapter")));
        }
        match &power.profiles {
            Profiles::Missing => rows.push(muted_row(&tr!(
                "Power modes aren't available because the power profiles service isn't installed."
            ))),
            Profiles::Offered(names) if names.is_empty() => {
                rows.push(muted_row(&tr!("This computer doesn't offer power modes.")));
            }
            Profiles::Offered(_) => {}
        }
        if !self.sleeps {
            rows.push(muted_row(&tr!("This computer can't sleep yet, so it stays on when idle. The display still turns off as set in Lock Screen.")));
        }
        rows
    }
}
