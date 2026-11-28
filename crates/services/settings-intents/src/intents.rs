use bande_intents::{Intent, List, Name, Ref, Refs, Text, Thing, Words};
use bande_l10n::text;

/// A setting of the desktop.
#[derive(Thing)]
#[thing(id = "settings.setting", title = text!("Setting"))]
pub struct Setting {
    pub reference: Ref<Self>,
    pub title: Name,
    #[thing(find)]
    pub value: Option<Text<4096>>,
    pub kind: Option<Form>,
    pub accepts: Option<List<Text<255>, 100>>,
    pub labels: Option<List<Text<255>, 100>>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub step: Option<i64>,
    #[thing(find)]
    pub pane: Option<Text<64>>,
    pub writable: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Words)]
pub enum Form {
    Switch,
    Choice,
    Number,
    Time,
    List,
}

/// Find settings, by words of their names or by the page they are on.
#[derive(Intent)]
#[intent(id = "settings.find", title = text!("Find Settings"), effects = [Read])]
pub struct Find {
    /// Words such as "dark mode"; every setting when absent.
    pub text: Option<Text<256>>,
    /// A page's word, such as "appearance".
    pub pane: Option<Text<64>>,
}

/// Get the values of settings and what they accept.
#[derive(Intent)]
#[intent(id = "settings.get", title = text!("Get Settings"), effects = [Read])]
pub struct Get {
    pub settings: Refs<Setting>,
    pub properties: List<Text<64>, 32>,
}

/// Show a page of Settings to the person.
#[derive(Intent)]
#[intent(id = "settings.open_pane", title = text!("Open Settings Page"), effects = [Show])]
pub struct OpenPane {
    /// The page's word, such as "appearance".
    pub pane: Text<64>,
}

/// Give a setting a new value.
#[derive(Intent)]
#[intent(
    id = "settings.set",
    title = text!("Change Setting"),
    effects = [Change, Disrupt, Critical],
    undo,
    phrases = [phrase(text!("{setting} {value}"))]
)]
pub struct Set {
    pub setting: Ref<Setting>,
    /// One of what the setting accepts: a word, on or off, a number, a time, or a list separated
    /// by commas.
    pub value: Text<4096>,
}

#[derive(Intent)]
#[intent(
    id = "settings.restore",
    title = text!("Change Setting"),
    effects = [Change, Disrupt, Critical]
)]
pub struct Restore {
    pub setting: Ref<Setting>,
    pub stored: Text<65536>,
}
