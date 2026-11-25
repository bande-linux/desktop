use std::sync::LazyLock;

use bande_intents::{List, Name, Ref, Refusal, Text, Wanted};
use bande_l10n::tr;
use bande_settings_panes::{
    Computer, Kind, List as Listed, Mark, Pane, PaneId, Setting, SettingId, Value, panes_of,
};
use bande_status::devices::Hardware;

use crate::intents::{self, Form};

const ON: &str = "on";
const OFF: &str = "off";
const PERCENT: f32 = 100.0;

static PANES: LazyLock<Vec<Pane>> = LazyLock::new(|| {
    let hardware = Hardware::scan();
    panes_of(Computer {
        battery: hardware.battery,
        wifi: hardware.wifi,
        bluetooth: hardware.bluetooth,
        sleeps: true,
    })
});

/// The panes of this computer.
pub fn panes() -> &'static [Pane] {
    &PANES
}

/// A setting that programs may see, and the page it is on.
pub struct Served {
    pub pane: PaneId,
    pub setting: &'static Setting,
}

fn is_time(id: SettingId) -> bool {
    matches!(
        id,
        SettingId::FocusFrom | SettingId::FocusTo | SettingId::NightFrom | SettingId::NightTo
    )
}

/// The settings that programs may set and this owner can read.
pub fn served() -> impl Iterator<Item = Served> {
    panes()
        .iter()
        .flat_map(|pane| {
            pane.settings().map(|setting| Served {
                pane: pane.id,
                setting,
            })
        })
        .filter(|served| {
            !matches!(served.id().mark(None), Mark::Hidden | Mark::Read)
                && served.form().is_some()
                && (served.id() == SettingId::DoNotDisturb
                    || bande_settings_client::CONFIG_SETTINGS.contains(&served.id()))
        })
}

impl Served {
    pub fn id(&self) -> SettingId {
        self.setting.id
    }

    pub fn reference(&self) -> Ref<intents::Setting> {
        Ref::new(self.id().word())
    }

    pub fn form(&self) -> Option<Form> {
        Some(match &self.setting.kind {
            Kind::Choices(_) | Kind::Menu(_) | Kind::Swatches(_) | Kind::Stops { .. } => {
                Form::Choice
            }
            Kind::Switch => Form::Switch,
            Kind::Level | Kind::Range { .. } => Form::Number,
            Kind::Text if is_time(self.id()) => Form::Time,
            Kind::List(Listed::InputSources | Listed::SearchExcluded) => Form::List,
            _ => return None,
        })
    }

    /// Each word it accepts, with its label.
    fn choices(&self) -> Vec<(String, String)> {
        match self.form() {
            Some(Form::Switch) => vec![(ON.to_owned(), tr!("On")), (OFF.to_owned(), tr!("Off"))],
            _ => self
                .setting
                .kind
                .choices()
                .iter()
                .filter_map(|choice| Some((word(&choice.value)?, choice.label.clone())))
                .collect(),
        }
    }

    fn range(&self) -> Option<(u32, u32, u32)> {
        match &self.setting.kind {
            Kind::Level => Some((0, PERCENT as u32, 1)),
            Kind::Range { range, step, .. } => Some((range.0, range.1, *step)),
            _ => None,
        }
    }

    /// `value` as a person reads it.
    pub fn label(&self, value: &Value) -> String {
        let written = word(value).unwrap_or_default();
        self.choices()
            .into_iter()
            .find(|(word, _)| *word == written)
            .map_or(written, |(_, label)| label)
    }

    /// The value written as `text`.
    pub fn parse(&self, text: &str) -> Result<Value, Refusal> {
        self.parsed(text)
            .ok_or_else(|| Refusal::invalid("value", self.accepts()))
    }

    fn parsed(&self, text: &str) -> Option<Value> {
        match self.form()? {
            Form::Switch => match text {
                ON => Some(Value::Flag(true)),
                OFF => Some(Value::Flag(false)),
                _ => None,
            },
            Form::Choice => self
                .setting
                .kind
                .choices()
                .iter()
                .find(|choice| word(&choice.value).as_deref() == Some(text))
                .map(|choice| choice.value.clone()),
            Form::Number => {
                let (least, most, step) = self.range()?;
                let number: u32 = text.parse().ok()?;
                let fits =
                    (least..=most).contains(&number) && (number - least).is_multiple_of(step);
                fits.then_some(match self.setting.kind {
                    Kind::Level => Value::Fraction(number as f32 / PERCENT),
                    _ => Value::Number(number),
                })
            }
            Form::Time => {
                let time: jiff::civil::Time = text.parse().ok()?;
                Some(Value::Text(time.strftime("%H:%M").to_string()))
            }
            Form::List => Some(Value::Codes(
                text.split(',')
                    .map(|item| item.trim().to_owned())
                    .filter(|item| !item.is_empty())
                    .collect(),
            )),
        }
    }

    fn accepts(&self) -> Vec<String> {
        if let Some((least, most, step)) = self.range() {
            return vec![tr!(
                "{minimum} to {maximum} in steps of {step}",
                minimum = least,
                maximum = most,
                step = step
            )];
        }
        match self.form() {
            Some(Form::Time) => vec!["22:30".to_owned()],
            Some(Form::List) => vec![tr!("a list separated by commas")],
            _ => self.choices().into_iter().map(|(word, _)| word).collect(),
        }
    }

    pub fn thing(&self, value: Option<&Value>, wanted: &Wanted) -> intents::Setting {
        let choices = self.choices();
        let listed = |texts: Vec<String>| {
            (!texts.is_empty()).then(|| List(texts.into_iter().map(Text::from).collect()))
        };
        let range = self.range();
        intents::Setting {
            reference: self.reference(),
            title: Name::new(&self.setting.label),
            value: value.and_then(word).map(Text::from),
            kind: self.form(),
            accepts: listed(choices.iter().map(|(word, _)| word.clone()).collect()),
            labels: match wanted.has("labels") {
                true => listed(choices.into_iter().map(|(_, label)| label).collect()),
                false => None,
            },
            minimum: range.map(|(least, ..)| i64::from(least)),
            maximum: range.map(|(_, most, _)| i64::from(most)),
            step: range.map(|(.., step)| i64::from(step)),
            pane: Some(self.pane.keyword().into()),
            writable: Some(true),
        }
    }
}

/// `value` as `settings.get` writes it.
fn word(value: &Value) -> Option<String> {
    Some(match value {
        Value::Keyword(word) | Value::Text(word) => word.clone(),
        Value::Seconds(seconds) => seconds.to_string(),
        Value::Number(number) => number.to_string(),
        Value::Flag(on) => if *on { ON } else { OFF }.to_owned(),
        Value::Fraction(fraction) => ((fraction * PERCENT).round() as u32).to_string(),
        Value::Codes(codes) => codes.join(", "),
        Value::Pictures { .. } => return None,
    })
}
