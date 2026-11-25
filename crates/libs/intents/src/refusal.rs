use std::collections::HashMap;
use std::fmt;

use bande_l10n::tr;
use zbus::zvariant;

use crate::Dict;

const PREFIX: &str = "org.bande.Intents.Error.";
const NO_REPLY: [&str; 2] = [
    "org.freedesktop.DBus.Error.NoReply",
    "org.freedesktop.DBus.Error.Disconnected",
];
const CANDIDATES: usize = 5;

/// Why an intent was not carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    UnknownIntent,
    InvalidArgument,
    NotFound,
    Ambiguous,
    NotGranted,
    Declined,
    Expired,
    Conflict,
    Interrupted,
    AccessDenied,
    Limited,
    Unavailable,
    Failed,
}

impl Reason {
    const ALL: [(Self, &'static str, &'static str); 13] = [
        (Self::UnknownIntent, "UnknownIntent", "unknown_intent"),
        (Self::InvalidArgument, "InvalidArgument", "invalid_argument"),
        (Self::NotFound, "NotFound", "not_found"),
        (Self::Ambiguous, "Ambiguous", "ambiguous"),
        (Self::NotGranted, "NotGranted", "not_granted"),
        (Self::Declined, "Declined", "declined"),
        (Self::Expired, "Expired", "expired"),
        (Self::Conflict, "Conflict", "conflict"),
        (Self::Interrupted, "Interrupted", "interrupted"),
        (Self::AccessDenied, "AccessDenied", "access_denied"),
        (Self::Limited, "Limited", "limited"),
        (Self::Unavailable, "Unavailable", "unavailable"),
        (Self::Failed, "Failed", "failed"),
    ];

    fn names(self) -> (&'static str, &'static str) {
        Self::ALL
            .iter()
            .find(|(reason, ..)| *reason == self)
            .map_or(("Failed", "failed"), |(_, name, word)| (*name, *word))
    }

    /// The name outside the bus: `not_found`.
    pub fn word(self) -> &'static str {
        self.names().1
    }
}

/// What an owner answers instead of a result: one sentence that says what to do, and what a
/// caller needs to try again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: Reason,
    pub message: String,
    pub parameter: Option<String>,
    pub accepts: Vec<String>,
    pub candidates: Vec<String>,
}

impl Refusal {
    pub fn new(reason: Reason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
            parameter: None,
            accepts: Vec::new(),
            candidates: Vec::new(),
        }
    }

    /// The argument `parameter` is missing, of another type, or not one of what is accepted.
    pub fn invalid(parameter: &str, accepts: Vec<String>) -> Self {
        let message = tr!(
            "“{parameter}” takes: {accepts}",
            parameter = parameter,
            accepts = accepts.join(", ")
        );
        Self {
            parameter: Some(parameter.to_owned()),
            accepts,
            ..Self::new(Reason::InvalidArgument, message)
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Reason::NotFound, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(Reason::Conflict, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(Reason::Unavailable, message)
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::new(Reason::Failed, message)
    }

    /// Adds up to five references that may have been meant; the owner gives only those within the
    /// caller's limits.
    pub fn candidates(mut self, candidates: impl IntoIterator<Item = String>) -> Self {
        self.candidates = candidates.into_iter().take(CANDIDATES).collect();
        self
    }

    pub(crate) fn interrupted() -> Self {
        Self::new(
            Reason::Interrupted,
            tr!("The program ended while it worked. The change may be there in part or whole."),
        )
    }

    pub(crate) fn details(&self) -> Dict {
        let mut details = Dict::new();
        let mut add = |name: &str, value: Option<zvariant::Value<'_>>| {
            if let Some(Ok(value)) = value.map(zvariant::OwnedValue::try_from) {
                details.insert(name.to_owned(), value);
            }
        };
        add(
            "parameter",
            self.parameter.as_deref().map(zvariant::Value::from),
        );
        let list = |words: &Vec<String>| (!words.is_empty()).then(|| words.clone().into());
        add("accepts", list(&self.accepts));
        add("candidates", list(&self.candidates));
        details
    }

    pub(crate) fn body(&self) -> (String, Dict) {
        (self.message.clone(), self.details())
    }

    pub(crate) fn from_parts(word: &str, message: String, mut details: Dict) -> Self {
        let reason = Reason::ALL
            .iter()
            .find(|(_, name, snake)| *name == word || *snake == word)
            .map_or(Reason::Failed, |(reason, ..)| *reason);
        let mut list = |name: &str| {
            details
                .remove(name)
                .and_then(|value| value.try_into().ok())
                .unwrap_or_default()
        };
        let (accepts, candidates) = (list("accepts"), list("candidates"));
        Self {
            reason,
            message,
            parameter: details
                .remove("parameter")
                .and_then(|value| value.try_into().ok()),
            accepts,
            candidates,
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Refusal {}

impl zbus::DBusError for Refusal {
    fn create_reply(&self, call: &zbus::message::Header<'_>) -> zbus::Result<zbus::Message> {
        zbus::Message::error(call, self.name())?.build(&self.body())
    }

    fn name(&self) -> zbus::names::ErrorName<'_> {
        zbus::names::ErrorName::from_string_unchecked(format!("{PREFIX}{}", self.reason.names().0))
    }

    fn description(&self) -> Option<&str> {
        Some(&self.message)
    }
}

/// An owner's answer read from the bus; an owner that is gone is `Unavailable`, one that ended
/// while it was asked `Interrupted`.
impl From<zbus::Error> for Refusal {
    fn from(error: zbus::Error) -> Self {
        if bande_bus::is_unowned(&error) {
            return Self::unavailable(error.to_string());
        }
        let zbus::Error::MethodError(name, text, reply) = &error else {
            return Self::failed(error.to_string());
        };
        if NO_REPLY.contains(&name.as_str()) {
            return Self::interrupted();
        }
        let Some(word) = name.as_str().strip_prefix(PREFIX) else {
            return Self::failed(text.clone().unwrap_or_else(|| name.to_string()));
        };
        let (message, details) = reply
            .body()
            .deserialize::<(String, HashMap<String, zvariant::OwnedValue>)>()
            .unwrap_or_else(|_| (text.clone().unwrap_or_default(), Dict::new()));
        Self::from_parts(word, message, details)
    }
}
