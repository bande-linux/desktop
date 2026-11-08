use bande_toolkit::ui::Toggle;

use crate::L10N;
use crate::battery::{BatteryCommand, Charge, ChargeLimit, Profiles};
use crate::general::info_row;
use crate::pages::{Widgets, boxed, muted_row};
use crate::view::{Intent, SettingsView, intent};
use crate::widgets::{LevelHistory, Row, Text, Tone};

const HEALTHY_CAPACITY: f64 = 80.0;

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
        let Some(battery) = self.power.as_ref().and_then(|power| power.battery.clone()) else {
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
        if let Some(capacity) = battery.capacity {
            let health = match capacity >= HEALTHY_CAPACITY {
                true => L10N.tr("battery-health-normal"),
                false => L10N.tr("battery-health-service"),
            };
            rows.push(boxed(
                Row::new(
                    Text::new(&L10N.tr("battery-health"), Tone::Primary),
                    Some(boxed(bande_toolkit::ui::Label::new(&health))),
                )
                .detail(
                    Text::new(
                        &L10N.tr_args("battery-capacity", [("percent", capacity.round().into())]),
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
            Some(end) => L10N.tr_args("charge-limit", [("percent", end.into())]),
            None => L10N.tr("charge-limit-any"),
        };
        let set = intent(&self.intents, |on| {
            Intent::Battery(BatteryCommand::ChargeLimit(on))
        });
        boxed(
            Row::new(
                Text::new(&label, Tone::Primary),
                Some(boxed(Toggle::new(&label, limit.enabled, set))),
            )
            .detail(Text::new(&L10N.tr("charge-limit-detail"), Tone::Muted).caption()),
        )
    }

    pub(crate) fn power_status_rows(&mut self) -> Widgets {
        let Some(power) = self.power.clone() else {
            return vec![muted_row(&L10N.tr("power-unavailable"))];
        };
        let mut rows = Vec::new();
        if power.battery.is_none() {
            rows.push(info_row(
                &L10N.tr("power-source"),
                &L10N.tr("power-on-adapter"),
            ));
        }
        match &power.profiles {
            Profiles::Missing => rows.push(muted_row(&L10N.tr("power-modes-missing"))),
            Profiles::Stopped => rows.push(muted_row(&L10N.tr("power-modes-stopped"))),
            Profiles::Offered(names) if names.is_empty() => {
                rows.push(muted_row(&L10N.tr("power-modes-none")));
            }
            Profiles::Offered(_) => {}
        }
        if !self.sleeps {
            rows.push(muted_row(&L10N.tr("power-no-sleep")));
        }
        rows
    }
}
