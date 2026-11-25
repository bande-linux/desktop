use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{Dict, Kind, Refusal, Value};

/// What an intent can do to the person's data or the world outside; it decides what is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Everything that is not foreign.
    Read,
    /// Text someone else could have chosen, also when it is only matched.
    ReadForeign,
    /// Puts something in front of the person.
    Show,
    /// A change only the person can see, that can be undone or done again without loss.
    Change,
    /// Leaves the computer, reaches a person, or writes where another can read.
    Send,
    /// May lose work, runs something, or may cut a connection.
    Disrupt,
    /// Destroys for good, administers, opens the computer or lowers a protection.
    Critical,
}

impl Effect {
    const ALL: [(Self, &'static str); 7] = [
        (Self::Read, "read"),
        (Self::ReadForeign, "read_foreign"),
        (Self::Show, "show"),
        (Self::Change, "change"),
        (Self::Send, "send"),
        (Self::Disrupt, "disrupt"),
        (Self::Critical, "critical"),
    ];

    pub fn word(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(effect, _)| *effect == self)
            .map_or("", |(_, word)| word)
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(_, candidate)| *candidate == word)
            .map(|(effect, _)| *effect)
    }
}

impl fmt::Display for Effect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.word())
    }
}

/// A text for people, kept in English and looked up in the catalog of the program that serves it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub en: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl From<bande_l10n::Text> for Label {
    fn from(text: bande_l10n::Text) -> Self {
        Self {
            en: text.english().to_owned(),
            context: text.context().map(str::to_owned),
        }
    }
}

/// How an intent is written in Search: words, and parameters in braces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Phrase {
    pub words: Label,
    /// Arguments the phrase fixes, as a person would write them.
    #[serde(default)]
    pub arguments: BTreeMap<String, String>,
}

impl Phrase {
    pub fn new(words: bande_l10n::Text, arguments: &[(&str, &str)]) -> Self {
        Self {
            words: words.into(),
            arguments: arguments
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(flatten)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
}

impl Parameter {
    pub fn of<T: Value>(name: &str, description: &str) -> Self {
        Self {
            name: name.to_owned(),
            description: description.to_owned(),
            kind: T::kind(),
            required: T::absent().is_none(),
        }
    }
}

/// What performing an intent gives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Returns {
    /// Things of this type.
    List(String),
    /// One computed value.
    Value(Kind),
    /// Computed dictionaries with these entries.
    Values(BTreeMap<String, Kind>),
}

/// An intent as its manifest describes it; its id is the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    pub title: Label,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub effects: Vec<Effect>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub undo: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phrases: Vec<Phrase>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<Returns>,
}

impl Definition {
    /// Whether it only reads, so that `find` and `get` answer it without a plan.
    pub fn only_reads(&self) -> bool {
        self.effects
            .iter()
            .all(|effect| matches!(effect, Effect::Read | Effect::ReadForeign))
    }

    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters
            .iter()
            .find(|parameter| parameter.name == name)
    }

    /// The arguments written in `json`, checked against the parameters.
    pub fn arguments_from_json(
        &self,
        id: &str,
        json: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Dict, Refusal> {
        json.iter()
            .map(|(name, value)| {
                let parameter = self
                    .parameter(name)
                    .ok_or_else(|| unknown(id, name, self.names()))?;
                let value = parameter
                    .kind
                    .from_json(value)
                    .ok_or_else(|| Refusal::invalid(name, parameter.kind.accepts()))?;
                Ok((name.clone(), value))
            })
            .collect()
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.parameters
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect()
    }
}

pub(crate) fn unknown(id: &str, name: &str, names: Vec<String>) -> Refusal {
    Refusal {
        message: bande_l10n::tr!(
            "{intent} has no “{parameter}”",
            intent = id,
            parameter = name
        ),
        ..Refusal::invalid(name, names)
    }
}

#[doc(hidden)]
pub fn reject_unknown(id: &str, arguments: &Dict, names: &[&str]) -> Result<(), Refusal> {
    match arguments
        .keys()
        .filter(|name| !names.contains(&name.as_str()))
        .min()
    {
        None => Ok(()),
        Some(name) => Err(unknown(
            id,
            name,
            names.iter().map(|name| (*name).to_owned()).collect(),
        )),
    }
}

/// A verb on things in one domain; derived with `#[derive(Intent)]`.
pub trait Intent: Sized + Send + 'static {
    const ID: &'static str;

    fn definition() -> Definition;

    fn decode(arguments: Dict) -> Result<Self, Refusal>;

    fn encode(self) -> Result<Dict, Refusal>;
}

/// The domain of an intent id, thing type, reference or receipt.
pub fn domain(id: &str) -> &str {
    id.split(['.', ':']).next().unwrap_or(id)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Property {
    #[serde(flatten)]
    pub kind: Kind,
    /// Someone else could have chosen its text.
    #[serde(default)]
    pub foreign: bool,
    /// `find` returns it.
    #[serde(default)]
    pub find: bool,
    /// `get` returns it only when it is named.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub content: bool,
}

impl Property {
    pub fn of<T: Value>(find: bool, foreign: bool, content: bool) -> Self {
        Self {
            kind: T::kind(),
            foreign,
            find,
            content,
        }
    }
}

/// A thing type as its manifest describes it; its type is the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThingDefinition {
    pub title: Label,
    /// The types its references are of, when it is a view of other things.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub view_of: Vec<String>,
    pub properties: BTreeMap<String, Property>,
}

/// A typed object with a reference and a title; derived with `#[derive(Thing)]`.
pub trait Thing: crate::Referent + Sized + Send + 'static {
    const TYPE: &'static str;

    fn definition() -> ThingDefinition;

    fn reference(&self) -> &str;

    /// The properties whose text someone else could have chosen in this thing, besides those that
    /// are foreign in every thing of the type.
    fn also_foreign(&self) -> Vec<&'static str>;

    /// The properties this thing has, the title among them.
    fn properties(self) -> Result<Dict, Refusal>;
}
