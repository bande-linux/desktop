use bande_datetime::daylight::{Place, Sky, sky, within_daily};
use bande_settings_panes::{SettingId, Value};
use jiff::Zoned;
use jiff::civil::Time;

use crate::L10N;
use crate::view::SettingsView;

pub const CUSTOM: &str = "custom";

pub fn clock_text(text: &str) -> Option<String> {
    let (hour, minute) = text.trim().split_once(':')?;
    let hour: u8 = hour.trim().parse().ok()?;
    let minute: u8 = minute.trim().parse().ok()?;
    (hour < 24 && minute < 60).then(|| format!("{hour:02}:{minute:02}"))
}

fn time(text: &str) -> Option<Time> {
    let text = clock_text(text)?;
    let (hour, minute) = text.split_once(':')?;
    Time::new(hour.parse().ok()?, minute.parse().ok()?, 0, 0).ok()
}

pub fn night_summary(
    on: bool,
    custom: Option<(&str, &str)>,
    now: &Zoned,
    place: Option<Place>,
) -> String {
    if !on {
        return L10N.tr("off");
    }
    let scheduled = match custom {
        Some((from, to)) => time(from)
            .zip(time(to))
            .and_then(|(from, to)| within_daily(now, from, to)),
        None => sky(now, place).map(|(sky, next)| (sky == Sky::Night, next)),
    };
    let Some((night, next)) = scheduled else {
        return L10N.tr("on");
    };
    let until = next
        .to_zoned(now.time_zone().clone())
        .strftime("%H:%M")
        .to_string();
    L10N.tr_args(
        if night { "on-until" } else { "off-until" },
        [("time", until.into())],
    )
}

impl SettingsView {
    fn text_value(&self, id: SettingId) -> Option<&str> {
        match self.values.get(&id)? {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn night_custom(&self) -> bool {
        self.values.get(&SettingId::NightSchedule) == Some(&Value::Keyword(CUSTOM.to_owned()))
    }

    pub(crate) fn night_light_summary(&self) -> Option<String> {
        let Value::Flag(on) = self.values.get(&SettingId::NightLight)? else {
            return None;
        };
        let custom = self
            .night_custom()
            .then(|| {
                Some((
                    self.text_value(SettingId::NightFrom)?,
                    self.text_value(SettingId::NightTo)?,
                ))
            })
            .flatten();
        let place = self.time.as_ref().and_then(|time| time.place);
        Some(night_summary(*on, custom, &(self.clock)(), place))
    }
}
