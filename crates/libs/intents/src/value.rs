use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::os::fd::AsFd;

use jiff::civil;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use zbus::zvariant::{self, Array, OwnedValue, Signature};

use crate::Refusal;

/// Arguments, properties and results as they travel on the bus.
pub type Dict = HashMap<String, OwnedValue>;

/// The most bytes of one text property that `find` and `get` return.
pub const SHOWN_TEXT: usize = 64 * 1024;
/// The most bytes of a [`RichText`].
pub const RICH_TEXT: usize = 1024 * 1024;
/// The most references in a list that is not a set.
pub const LISTED: usize = 1000;
/// The most things in a set.
pub const IN_SET: usize = 10_000;
const NAME: usize = 80;
const ROWS: usize = 16 * 1024 * 1024;
const ADDRESS: usize = 998;
const URL: usize = 8 * 1024;
pub(crate) const SET_PREFIX: &str = "set:";

/// What a parameter, property or result is, as manifests write it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Kind {
    Text {
        limit: usize,
    },
    Name,
    RichText,
    Rows,
    Bool,
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        minimum: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        maximum: Option<i64>,
    },
    Number,
    Date,
    Time,
    DateTime,
    DateOrDateTime,
    Duration,
    Url,
    Uri,
    Location,
    Address,
    Picture,
    Enum {
        words: Vec<String>,
    },
    Ref {
        of: Vec<String>,
    },
    List {
        items: Box<Kind>,
        limit: usize,
    },
}

impl Kind {
    /// What a value of this kind looks like, for an error and for help.
    pub fn accepts(&self) -> Vec<String> {
        let one = match self {
            Self::Text { limit } => format!("text of at most {limit} bytes"),
            Self::Name => format!("one line of at most {NAME} characters"),
            Self::RichText => format!("Markdown of at most {RICH_TEXT} bytes"),
            Self::Rows => "CSV".to_owned(),
            Self::Bool => return vec!["true".to_owned(), "false".to_owned()],
            Self::Integer { minimum, maximum } => match (minimum, maximum) {
                (Some(minimum), Some(maximum)) => format!("{minimum} to {maximum}"),
                _ => "a whole number".to_owned(),
            },
            Self::Number => "a number".to_owned(),
            Self::Date => "2026-11-24".to_owned(),
            Self::Time => "07:30".to_owned(),
            Self::DateTime => "2026-11-24T13:00:00+01:00[Europe/Stockholm]".to_owned(),
            Self::DateOrDateTime => {
                return [Self::Date, Self::DateTime]
                    .iter()
                    .flat_map(Self::accepts)
                    .collect();
            }
            Self::Duration => "PT25M".to_owned(),
            Self::Url => "https://example.org/".to_owned(),
            Self::Uri => "sftp://example.org/".to_owned(),
            Self::Location => "geo:59.33,18.07;u=100".to_owned(),
            Self::Address => "Anna Ek <anna@example.org>".to_owned(),
            Self::Picture => "a PNG".to_owned(),
            Self::Enum { words } => return words.clone(),
            Self::Ref { of } => return of.iter().map(|kind| format!("{kind}:…")).collect(),
            Self::List { items, limit } => {
                format!("at most {limit} of {}", items.accepts().join(", "))
            }
        };
        vec![one]
    }

    /// The D-Bus value written as `json`, or nothing when it is of another type.
    pub fn from_json(&self, json: &Json) -> Option<OwnedValue> {
        let value: zvariant::Value<'static> = match self {
            Self::Bool => json.as_bool()?.into(),
            Self::Integer { .. } => json.as_i64()?.into(),
            Self::Number => json.as_f64()?.into(),
            Self::Picture => return None,
            Self::List { items, .. } => {
                let mut array = Array::new(&items.signature());
                for item in json.as_array()? {
                    array.append(items.from_json(item)?.into()).ok()?;
                }
                array.into()
            }
            _ => json.as_str()?.to_owned().into(),
        };
        value.try_into().ok()
    }

    /// The D-Bus value a person wrote as `words` on a command line; a word without a type gets
    /// the one its reference needs.
    pub fn from_words(&self, words: &[String]) -> Option<OwnedValue> {
        let json = match self {
            Self::List { items, .. } => Json::Array(
                words
                    .iter()
                    .map(|word| items.json_from_word(word))
                    .collect::<Option<_>>()?,
            ),
            _ => match words {
                [word] => self.json_from_word(word)?,
                _ => return None,
            },
        };
        self.from_json(&json)
    }

    fn json_from_word(&self, word: &str) -> Option<Json> {
        Some(match self {
            Self::Bool => Json::Bool(match word {
                "on" | "true" | "yes" => true,
                "off" | "false" | "no" => false,
                _ => return None,
            }),
            Self::Integer { .. } => Json::from(word.parse::<i64>().ok()?),
            Self::Number => Json::from(word.parse::<f64>().ok()?),
            Self::Picture | Self::List { .. } => return None,
            Self::Ref { of } => {
                let typed = word.starts_with(SET_PREFIX)
                    || of.iter().any(|kind| reference_token(word, kind).is_some());
                match (typed, of.first()) {
                    (false, Some(kind)) => Json::String(format!("{kind}:{word}")),
                    _ => Json::String(word.to_owned()),
                }
            }
            _ => Json::String(word.to_owned()),
        })
    }

    fn signature(&self) -> Signature {
        match self {
            Self::Bool => Signature::Bool,
            Self::Integer { .. } => Signature::I64,
            Self::Number => Signature::F64,
            Self::Picture => Signature::Fd,
            _ => Signature::Str,
        }
    }
}

/// `value` as JSON, for a person, a tool or a file; a descriptor has none.
pub fn to_json(value: &zvariant::Value<'_>) -> Json {
    use zvariant::Value as V;
    match value {
        V::Bool(on) => Json::from(*on),
        V::U8(number) => Json::from(*number),
        V::I16(number) => Json::from(*number),
        V::U16(number) => Json::from(*number),
        V::I32(number) => Json::from(*number),
        V::U32(number) => Json::from(*number),
        V::I64(number) => Json::from(*number),
        V::U64(number) => Json::from(*number),
        V::F64(number) => Json::from(*number),
        V::Str(text) => Json::from(text.as_str()),
        V::ObjectPath(path) => Json::from(path.as_str()),
        V::Value(inner) => to_json(inner),
        V::Array(items) => items.iter().map(to_json).collect(),
        V::Dict(entries) => Json::Object(
            entries
                .iter()
                .filter_map(|(key, value)| match key {
                    V::Str(key) => Some((key.as_str().to_owned(), to_json(value))),
                    _ => None,
                })
                .collect(),
        ),
        _ => Json::Null,
    }
}

/// `dict` as a JSON object with its keys in order.
pub fn dict_to_json(dict: &Dict) -> Json {
    Json::Object(
        dict.iter()
            .map(|(name, value)| (name.clone(), to_json(value)))
            .collect(),
    )
}

/// A type a parameter, property or result can have.
pub trait Value: Sized {
    fn kind() -> Kind;

    /// `None` when the value is of another type, too long or not one of the words.
    fn decode(value: OwnedValue) -> Option<Self>;

    fn encode(self) -> Option<zvariant::Value<'static>>;

    /// What an absent argument means; `None` when it is required.
    fn absent() -> Option<Self> {
        None
    }
}

#[doc(hidden)]
pub fn take<T: Value>(arguments: &mut Dict, name: &str) -> Result<T, Refusal> {
    match arguments.remove(name) {
        Some(value) => T::decode(value),
        None => T::absent(),
    }
    .ok_or_else(|| Refusal::invalid(name, T::kind().accepts()))
}

#[doc(hidden)]
pub fn give<T: Value>(values: &mut Dict, name: &str, value: T) -> Result<(), Refusal> {
    if let Some(value) = value.encode() {
        values.insert(
            name.to_owned(),
            value
                .try_into()
                .map_err(|error: zvariant::Error| Refusal::failed(error.to_string()))?,
        );
    }
    Ok(())
}

macro_rules! plain {
    ($type:ty, $kind:expr) => {
        impl Value for $type {
            fn kind() -> Kind {
                $kind
            }

            fn decode(value: OwnedValue) -> Option<Self> {
                value.try_into().ok()
            }

            fn encode(self) -> Option<zvariant::Value<'static>> {
                Some(self.into())
            }
        }
    };
}

plain!(
    i64,
    Kind::Integer {
        minimum: None,
        maximum: None
    }
);
plain!(f64, Kind::Number);

impl Value for bool {
    fn kind() -> Kind {
        Kind::Bool
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        value.try_into().ok()
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.into())
    }

    fn absent() -> Option<Self> {
        Some(false)
    }
}

/// A whole number from `MIN` to `MAX`, such as [`Level`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Between<const MIN: i64, const MAX: i64>(pub i64);

/// A level: 0 to 100.
pub type Level = Between<0, 100>;

impl<const MIN: i64, const MAX: i64> Value for Between<MIN, MAX> {
    fn kind() -> Kind {
        Kind::Integer {
            minimum: Some(MIN),
            maximum: Some(MAX),
        }
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        let number = i64::try_from(value).ok()?;
        (MIN..=MAX).contains(&number).then_some(Self(number))
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.0.into())
    }
}

macro_rules! textual {
    ($(#[$doc:meta])* $type:ident, $kind:expr, $check:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $type(pub String);

        impl Value for $type {
            fn kind() -> Kind {
                $kind
            }

            fn decode(value: OwnedValue) -> Option<Self> {
                let text = String::try_from(value).ok()?;
                let check: fn(&str) -> bool = $check;
                check(&text).then_some(Self(text))
            }

            fn encode(self) -> Option<zvariant::Value<'static>> {
                Some(self.0.into())
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

textual!(
    /// Markdown its owner parses and writes out again with `bande-rich-text`.
    RichText,
    Kind::RichText,
    |text| text.len() <= RICH_TEXT && plain_text(text)
);
textual!(
    /// CSV (RFC 4180).
    Rows,
    Kind::Rows,
    |text| text.len() <= ROWS
);
textual!(
    /// An `https` or `http` URL without user information.
    Url,
    Kind::Url,
    |text| matches!(scheme(text), Some("https" | "http")) && bare_authority(text)
);
textual!(
    /// A URI of another scheme an intent names.
    Uri,
    Kind::Uri,
    |text| scheme(text).is_some() && !text.contains(is_separating)
);
textual!(
    /// A `geo:` URI (RFC 5870).
    Location,
    Kind::Location,
    |text| text.strip_prefix("geo:").is_some_and(|rest| {
        let mut numbers = rest.split(';').next().unwrap_or_default().split(',');
        let within = |number: Option<&str>, most: f64| {
            number
                .and_then(|number| number.parse::<f64>().ok())
                .is_some_and(|number| number.abs() <= most)
        };
        within(numbers.next(), 90.0) && within(numbers.next(), 180.0)
    })
);
textual!(
    /// A mail address (RFC 5322 and RFC 6532), with or without a name.
    Address,
    Kind::Address,
    |text| {
        let bare = match text.rsplit_once('<') {
            Some((_, rest)) => rest.strip_suffix('>').unwrap_or_default(),
            None => text,
        };
        text.len() <= ADDRESS
            && !text.contains(char::is_control)
            && bare
                .rsplit_once('@')
                .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
            && !bare.contains(is_separating)
    }
);

fn is_separating(character: char) -> bool {
    character.is_whitespace() || character.is_control()
}

fn plain_text(text: &str) -> bool {
    !text.contains('\0')
}

fn scheme(text: &str) -> Option<&str> {
    let (scheme, rest) = text.split_once(':')?;
    let valid = text.len() <= URL
        && scheme.starts_with(|first: char| first.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "+-.".contains(character))
        && !rest.is_empty();
    valid.then_some(scheme)
}

fn bare_authority(text: &str) -> bool {
    text.split_once("://").is_some_and(|(_, rest)| {
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        !authority.is_empty()
            && !authority.contains(['@', '\\'])
            && !text.contains(is_separating)
            && !text.contains('\\')
    })
}

/// Plain text of at most `LIMIT` bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Text<const LIMIT: usize>(pub String);

impl<const LIMIT: usize> Value for Text<LIMIT> {
    fn kind() -> Kind {
        Kind::Text { limit: LIMIT }
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        let text = String::try_from(value).ok()?;
        (text.len() <= LIMIT && plain_text(&text)).then_some(Self(text))
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.0.into())
    }
}

impl<const LIMIT: usize> fmt::Display for Text<LIMIT> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<const LIMIT: usize> From<String> for Text<LIMIT> {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl<const LIMIT: usize> From<&str> for Text<LIMIT> {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

/// A name: one line of at most 80 characters that cannot pose as anything else, so it is never
/// foreign. Characters a name cannot have are left out when it is made.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Name(String);

impl Name {
    pub fn new(text: &str) -> Self {
        Self(
            text.chars()
                .filter(|character| !character.is_control() && !is_bidirectional(*character))
                .take(NAME)
                .collect(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_bidirectional(character: char) -> bool {
    matches!(
        character,
        '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}

impl Value for Name {
    fn kind() -> Kind {
        Kind::Name
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        let text = String::try_from(value).ok()?;
        let name = Self::new(&text);
        (name.0 == text).then_some(name)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.0.into())
    }
}

fn parsed<T: std::str::FromStr>(value: OwnedValue) -> Option<T> {
    String::try_from(value).ok()?.parse().ok()
}

macro_rules! written {
    ($type:ty, $kind:expr) => {
        impl Value for $type {
            fn kind() -> Kind {
                $kind
            }

            fn decode(value: OwnedValue) -> Option<Self> {
                parsed(value)
            }

            fn encode(self) -> Option<zvariant::Value<'static>> {
                Some(self.to_string().into())
            }
        }
    };
}

written!(civil::Date, Kind::Date);
written!(jiff::Span, Kind::Duration);

impl Value for civil::Time {
    fn kind() -> Kind {
        Kind::Time
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        parsed(value)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.strftime("%H:%M").to_string().into())
    }
}

impl Value for jiff::Zoned {
    fn kind() -> Kind {
        Kind::DateTime
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        zoned(&String::try_from(value).ok()?)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.to_string().into())
    }
}

fn zoned(text: &str) -> Option<jiff::Zoned> {
    text.parse().ok().or_else(|| {
        let instant: jiff::Timestamp = text.parse().ok()?;
        Some(instant.to_zoned(jiff::tz::TimeZone::system()))
    })
}

/// A day, or an instant.
#[derive(Debug, Clone, PartialEq)]
pub enum DateOrDateTime {
    Date(civil::Date),
    DateTime(jiff::Zoned),
}

impl Value for DateOrDateTime {
    fn kind() -> Kind {
        Kind::DateOrDateTime
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        let text = String::try_from(value).ok()?;
        match text.contains('T') {
            true => zoned(&text).map(Self::DateTime),
            false => text.parse().ok().map(Self::Date),
        }
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        match self {
            Self::Date(date) => date.encode(),
            Self::DateTime(instant) => instant.encode(),
        }
    }
}

/// A PNG, as a descriptor.
#[derive(Debug)]
pub struct Picture(pub std::os::fd::OwnedFd);

impl Value for Picture {
    fn kind() -> Kind {
        Kind::Picture
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        let descriptor = zvariant::Fd::try_from(zvariant::Value::from(value)).ok()?;
        Some(Self(descriptor.as_fd().try_clone_to_owned().ok()?))
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(zvariant::Fd::from(self.0).into())
    }
}

/// A closed list of words; derived for an enum with `#[derive(Words)]`.
pub trait Words: Sized + 'static {
    const WORDS: &'static [&'static str];

    fn word(&self) -> &'static str;

    fn from_word(word: &str) -> Option<Self>;
}

impl<T: Words> Value for T {
    fn kind() -> Kind {
        Kind::Enum {
            words: T::WORDS.iter().map(|word| (*word).to_owned()).collect(),
        }
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        T::from_word(&String::try_from(value).ok()?)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.word().into())
    }
}

/// The things a [`Ref`] can name: one thing type, a tuple of them, or a type of another program,
/// named by a unit struct.
pub trait Referent: 'static {
    const TYPES: &'static [&'static str];
}

impl<A: crate::Thing, B: crate::Thing> Referent for (A, B) {
    const TYPES: &'static [&'static str] = &[A::TYPE, B::TYPE];
}

impl<A: crate::Thing, B: crate::Thing, C: crate::Thing> Referent for (A, B, C) {
    const TYPES: &'static [&'static str] = &[A::TYPE, B::TYPE, C::TYPE];
}

/// The token of `reference` when it is of the type `kind`.
pub fn reference_token<'a>(reference: &'a str, kind: &str) -> Option<&'a str> {
    reference
        .strip_prefix(kind)?
        .strip_prefix(':')
        .filter(|token| !token.is_empty())
}

/// The name of one thing of the type `T`; it gives no access by itself.
pub struct Ref<T> {
    text: String,
    thing: PhantomData<fn() -> T>,
}

impl<T: Referent> Ref<T> {
    /// The reference with this token, of the first type `T` names.
    pub fn new(token: &str) -> Self {
        let kind = T::TYPES.first().copied().unwrap_or_default();
        Self {
            text: format!("{kind}:{token}"),
            thing: PhantomData,
        }
    }

    /// `text` when it is a reference to what `T` names.
    pub fn parse(text: &str) -> Option<Self> {
        T::TYPES
            .iter()
            .any(|kind| reference_token(text, kind).is_some())
            .then(|| Self {
                text: text.to_owned(),
                thing: PhantomData,
            })
    }
}

impl<T> Ref<T> {
    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn token(&self) -> &str {
        self.text
            .split_once(':')
            .map_or(self.text.as_str(), |(_, token)| token)
    }

    /// The thing's type, for a reference that may be of several.
    pub fn thing_type(&self) -> &str {
        self.text
            .split_once(':')
            .map_or(self.text.as_str(), |(kind, _)| kind)
    }
}

impl<T> Clone for Ref<T> {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone(),
            thing: PhantomData,
        }
    }
}

impl<T> PartialEq for Ref<T> {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl<T> Eq for Ref<T> {}

impl<T> fmt::Debug for Ref<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl<T> fmt::Display for Ref<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl<T: Referent> Value for Ref<T> {
    fn kind() -> Kind {
        Kind::Ref {
            of: T::TYPES.iter().map(|kind| (*kind).to_owned()).collect(),
        }
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        Self::parse(&String::try_from(value).ok()?)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        Some(self.text.into())
    }
}

impl<T: Value> Value for Option<T> {
    fn kind() -> Kind {
        T::kind()
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        T::decode(value).map(Some)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        self.and_then(T::encode)
    }

    fn absent() -> Option<Self> {
        Some(None)
    }
}

fn decode_items<T: Value>(value: OwnedValue, limit: usize) -> Option<Vec<T>> {
    let items = Array::try_from(value).ok()?;
    if items.len() > limit {
        return None;
    }
    items
        .iter()
        .map(|item| T::decode(item.try_to_owned().ok()?))
        .collect()
}

fn encode_items<T: Value>(items: Vec<T>) -> Option<zvariant::Value<'static>> {
    let mut array = Array::new(&T::kind().signature());
    for item in items {
        array.append(item.encode()?).ok()?;
    }
    Some(array.into())
}

/// At most `LIMIT` values; absent is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List<T, const LIMIT: usize>(pub Vec<T>);

impl<T: Value, const LIMIT: usize> Value for List<T, LIMIT> {
    fn kind() -> Kind {
        Kind::List {
            items: Box::new(T::kind()),
            limit: LIMIT,
        }
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        decode_items(value, LIMIT).map(Self)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        encode_items(self.0)
    }

    fn absent() -> Option<Self> {
        Some(Self(Vec::new()))
    }
}

/// The things an intent acts on: at most 1,000 references, or everything in a set that `find`
/// gave; an intent needs at least one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refs<T>(pub Vec<Ref<T>>);

impl<T: Referent> Value for Refs<T> {
    fn kind() -> Kind {
        Kind::List {
            items: Box::new(Ref::<T>::kind()),
            limit: LISTED,
        }
    }

    fn decode(value: OwnedValue) -> Option<Self> {
        decode_items(value, IN_SET)
            .filter(|references: &Vec<_>| !references.is_empty())
            .map(Self)
    }

    fn encode(self) -> Option<zvariant::Value<'static>> {
        encode_items(self.0)
    }
}
