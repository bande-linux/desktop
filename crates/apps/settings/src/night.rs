use bande_l10n::{text, tr};

use crate::view::SettingsView;
use bande_datetime::DateFormats;
use bande_datetime::daylight::{Place, Sky, sky, within_daily};
use bande_settings_panes::{SettingId, Value};
use jiff::civil::Time;
use jiff::{Timestamp, Zoned};

const CUSTOM: &str = "custom";

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

fn shown(formats: &DateFormats, next: Timestamp, now: &Zoned) -> String {
    formats.clock_time(&next.to_zoned(now.time_zone().clone()))
}

fn night_summary(
    formats: &DateFormats,
    custom: Option<(&str, &str)>,
    now: &Zoned,
    place: Option<Place>,
) -> Option<String> {
    let (night, next) = match custom {
        Some((from, to)) => time(from)
            .zip(time(to))
            .and_then(|(from, to)| within_daily(now, from, to)),
        None => sky(now, place).map(|(sky, next)| (sky == Sky::Night, next)),
    }?;
    let until = shown(formats, next, now);
    Some(
        (if night {
            text!("On until {time}")
        } else {
            text!("Off until {time}")
        })
        .tr_args(&[("time", &until as &dyn std::fmt::Display)]),
    )
}

impl SettingsView {
    fn text_value(&self, id: SettingId) -> Option<&str> {
        match self.values.get(&id)? {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub(crate) fn do_not_disturb_summary(&self) -> Option<String> {
        let on = self.values.get(&SettingId::DoNotDisturb) == Some(&Value::Flag(true));
        let now = (self.clock)();
        let scheduled = (self.values.get(&SettingId::FocusScheduled) == Some(&Value::Flag(true)))
            .then(|| {
                let from = time(self.text_value(SettingId::FocusFrom)?)?;
                let to = time(self.text_value(SettingId::FocusTo)?)?;
                within_daily(&now, from, to)
            })
            .flatten();
        match (on, scheduled) {
            (true, Some((true, next))) => Some(tr!(
                "Do Not Disturb is on until {time}",
                time = shown(&self.date_formats(), next, &now)
            )),
            (true, _) => Some(tr!("Do Not Disturb is on")),
            (false, Some((false, next))) => Some(tr!(
                "Do Not Disturb turns on at {time}",
                time = shown(&self.date_formats(), next, &now)
            )),
            (false, _) => None,
        }
    }

    fn night_on(&self) -> bool {
        self.values.get(&SettingId::NightLight) == Some(&Value::Flag(true))
    }

    fn night_custom(&self) -> bool {
        self.values.get(&SettingId::NightSchedule) == Some(&Value::Keyword(CUSTOM.to_owned()))
    }

    pub(crate) fn night_revealed(&self, id: SettingId) -> SettingId {
        match id {
            SettingId::NightSchedule
            | SettingId::NightFrom
            | SettingId::NightTo
            | SettingId::NightWarmth
                if !self.night_on() =>
            {
                SettingId::NightLight
            }
            SettingId::NightFrom | SettingId::NightTo if !self.night_custom() => {
                SettingId::NightSchedule
            }
            _ => id,
        }
    }

    pub(crate) fn night_light_note(&self, id: SettingId) -> Option<String> {
        if id != SettingId::NightLight || !self.night_on() {
            return None;
        }
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
        night_summary(&self.date_formats(), custom, &(self.clock)(), place)
    }
}
