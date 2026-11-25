//! The phrases of installed intents, matched against one typed line.

use std::collections::{BTreeMap, HashMap, HashSet};

use bande_l10n::tr;

use crate::client::text;
use crate::{
    Client, Definition, Dict, Effect, Kind, Manifest, Manifests, Property, Reason, Refusal,
    Returns, plan, to_json,
};

const FEWEST_CHARACTERS: usize = 2;
const MOST_WORDS: usize = 8;
const MOST_FOUND: u32 = 20;
const MOST_OFFERED: usize = 3;
const ACCEPTS: &str = "accepts";
const LABELS: &str = "labels";
const SWITCH: [&str; 2] = ["on", "off"];

/// One thing that can be done as typed.
#[derive(Debug)]
pub struct Offer {
    /// What it does, such as "Appearance: Light".
    pub title: String,
    /// The intent's title.
    pub detail: String,
    /// The owner's program, whose app it belongs to.
    pub bus: String,
    pub intent: String,
    pub arguments: Dict,
}

impl Offer {
    /// Does it, and gives the summary and the receipt that undoes it.
    pub fn perform(&self, client: &Client) -> Result<(String, Option<String>), Refusal> {
        let prepared = client.prepare(&self.bus, &self.intent, &self.arguments)?;
        let summary = prepared.summary.clone();
        let performed = client.perform(prepared)?;
        Ok((summary, performed.receipt().map(str::to_owned)))
    }
}

enum Token {
    Literal(String),
    Placeholder(String),
}

struct Served {
    bus: String,
    intent: String,
    title: String,
    definition: Definition,
    tokens: Vec<Token>,
    fixed: BTreeMap<String, String>,
}

struct Findable {
    bus: String,
    find: String,
    get: Option<String>,
    accepts: bool,
}

/// The phrases of every manifest, in the person's language.
pub struct Phrases {
    served: Vec<Served>,
    things: HashMap<String, Findable>,
}

fn tokens(words: &str) -> Vec<Token> {
    words
        .split_whitespace()
        .map(|word| {
            match word
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            {
                Some(name) => Token::Placeholder(name.to_owned()),
                None => Token::Literal(word.to_lowercase()),
            }
        })
        .collect()
}

fn served(manifest: &Manifest) -> Vec<Served> {
    if manifest
        .intents
        .values()
        .all(|definition| definition.phrases.is_empty())
    {
        return Vec::new();
    }
    let catalogs = manifest.catalogs();
    let mut all = Vec::new();
    for (id, definition) in &manifest.intents {
        for phrase in &definition.phrases {
            let label =
                |label: &crate::Label| catalogs.pattern(label.context.as_deref(), &label.en);
            all.push(Served {
                bus: manifest.bus.clone(),
                intent: id.clone(),
                title: label(&definition.title),
                definition: definition.clone(),
                tokens: tokens(&label(&phrase.words)),
                fixed: phrase.arguments.clone(),
            });
        }
    }
    all
}

fn findable(
    manifest: &Manifest,
    kind: &str,
    properties: &BTreeMap<String, Property>,
) -> Option<Findable> {
    let gives =
        |definition: &Definition| definition.returns == Some(Returns::List(kind.to_owned()));
    let with = |parameter: &str| {
        manifest
            .intents
            .iter()
            .find(|(id, definition)| {
                id.split_once('.').is_some_and(|(_, verb)| {
                    verb == parameter || verb.starts_with(&format!("{parameter}_"))
                }) && gives(definition)
            })
            .map(|(id, definition)| (id.clone(), definition))
    };
    let (find, definition) = with("find")?;
    definition.parameter("text")?;
    Some(Findable {
        bus: manifest.bus.clone(),
        find,
        get: with("get").map(|(id, _)| id),
        accepts: properties.contains_key(ACCEPTS),
    })
}

struct Chosen {
    value: zbus::zvariant::OwnedValue,
    shown: String,
    follows_thing: bool,
}

struct Attempt<'a> {
    phrases: &'a Phrases,
    client: &'a Client,
    wanted: &'a dyn Fn() -> bool,
    silent: HashSet<String>,
    found: HashMap<(String, String), Vec<Dict>>,
}

impl Phrases {
    pub fn new(manifests: &Manifests) -> Self {
        Self {
            served: manifests.iter().flat_map(served).collect(),
            things: manifests
                .iter()
                .flat_map(|manifest| {
                    manifest.things.iter().filter_map(|(kind, thing)| {
                        Some((kind.clone(), findable(manifest, kind, &thing.properties)?))
                    })
                })
                .collect(),
        }
    }

    /// Whether no installed program has a phrase.
    pub fn is_empty(&self) -> bool {
        self.served.is_empty()
    }

    /// What can be done as `typed`: at most three, each prepared once to see that it can, and
    /// nothing critical. Those plans end with the connection of `client`, so it is one of its own.
    /// An owner that does not answer once is not asked again, and nobody is when `wanted` says that
    /// something else has been typed.
    pub fn offers(&self, client: &Client, typed: &str, wanted: &dyn Fn() -> bool) -> Vec<Offer> {
        let words: Vec<String> = typed.split_whitespace().map(str::to_lowercase).collect();
        if typed.trim().chars().count() < FEWEST_CHARACTERS || words.len() > MOST_WORDS {
            return Vec::new();
        }
        let original: Vec<&str> = typed.split_whitespace().collect();
        let mut attempt = Attempt {
            phrases: self,
            client,
            wanted,
            silent: HashSet::new(),
            found: HashMap::new(),
        };
        let mut offers: Vec<Offer> = Vec::new();
        for phrase in &self.served {
            for offer in attempt.offers(phrase, &words, &original) {
                let known = offers
                    .iter()
                    .any(|known| known.title == offer.title && known.intent == offer.intent);
                if offers.len() < MOST_OFFERED && !known && attempt.can_be_done(&offer) {
                    offers.push(offer);
                }
            }
        }
        offers
    }
}

fn spread<'a>(
    tokens: &'a [Token],
    words: std::ops::Range<usize>,
    typed: &[String],
) -> Vec<Vec<(&'a str, std::ops::Range<usize>)>> {
    match tokens.split_first() {
        None => match words.is_empty() {
            true => vec![Vec::new()],
            false => Vec::new(),
        },
        Some((Token::Literal(literal), rest)) => match typed.get(words.start) {
            Some(word) if !words.is_empty() && word == literal => {
                spread(rest, words.start + 1..words.end, typed)
            }
            _ => Vec::new(),
        },
        Some((Token::Placeholder(name), rest)) => (words.start + 1..=words.end)
            .flat_map(|end| {
                spread(rest, end..words.end, typed)
                    .into_iter()
                    .map(move |mut tail| {
                        tail.insert(0, (name.as_str(), words.start..end));
                        tail
                    })
            })
            .collect(),
    }
}

impl Attempt<'_> {
    fn asks(&self, bus: &str) -> bool {
        (self.wanted)() && !self.silent.contains(bus)
    }

    fn heard<T>(&mut self, bus: &str, answer: Result<T, Refusal>) -> Option<T> {
        answer
            .inspect_err(|refusal| {
                tracing::debug!("{bus} offers nothing: {refusal}");
                if matches!(refusal.reason, Reason::Interrupted | Reason::Unavailable) {
                    self.silent.insert(bus.to_owned());
                }
            })
            .ok()
    }

    fn can_be_done(&mut self, offer: &Offer) -> bool {
        if !self.asks(&offer.bus) {
            return false;
        }
        let prepared = self
            .client
            .prepare(&offer.bus, &offer.intent, &offer.arguments);
        match prepared {
            Ok(prepared) => !prepared.effects.contains(&Effect::Critical),
            Err(refusal) if refusal.reason == Reason::Interrupted => {
                self.heard::<()>(&offer.bus, Err(refusal));
                false
            }
            Err(_) => false,
        }
    }

    fn offers(&mut self, phrase: &Served, words: &[String], original: &[&str]) -> Vec<Offer> {
        let mut spreads = spread(&phrase.tokens, 0..words.len(), words);
        let placeholders: Vec<&str> = phrase
            .tokens
            .iter()
            .filter_map(|token| match token {
                Token::Placeholder(name) => Some(name.as_str()),
                Token::Literal(_) => None,
            })
            .collect();
        if placeholders.len() == phrase.tokens.len() && placeholders.len() == 2 {
            spreads.insert(
                0,
                placeholders
                    .iter()
                    .map(|name| (*name, 0..words.len()))
                    .collect(),
            );
        }
        let mut offers = Vec::new();
        for spread in spreads {
            if offers.len() >= MOST_OFFERED {
                break;
            }
            offers.extend(self.fill(phrase, &spread, words, original));
        }
        offers
    }

    fn fill(
        &mut self,
        phrase: &Served,
        spread: &[(&str, std::ops::Range<usize>)],
        words: &[String],
        original: &[&str],
    ) -> Vec<Offer> {
        let shared = spread.len() > 1 && spread.iter().all(|(_, range)| range.len() == words.len());
        let thing = spread.iter().find_map(|(name, range)| {
            match &phrase.definition.parameter(name)?.kind {
                Kind::Ref { of } => Some((*name, of.first()?.clone(), range.clone())),
                _ => None,
            }
        });
        let things = match &thing {
            Some((_, kind, range)) => self.find(kind, &words[range.clone()].join(" ")),
            None if shared => return Vec::new(),
            None => vec![Dict::new()],
        };
        let chooses = thing.as_ref().is_some_and(|(_, kind, _)| {
            let findable = self.phrases.things.get(kind);
            findable.is_some_and(|findable| findable.accepts)
        });
        let mut offers = Vec::new();
        for found in things {
            let chosen: Option<BTreeMap<&str, Chosen>> = spread
                .iter()
                .map(|(name, range)| {
                    let is_thing = thing.as_ref().is_some_and(|(of, ..)| of == name);
                    let chosen = match is_thing {
                        true => Some(Chosen {
                            value: plan::owned_text(text(&found, "reference")),
                            shown: text(&found, "title").to_owned(),
                            follows_thing: false,
                        }),
                        false => value(
                            &phrase.definition.parameter(name)?.kind,
                            &words[range.clone()],
                            &original[range.clone()],
                            range.end == words.len() && !chooses,
                            &found,
                            shared,
                        ),
                    };
                    Some((*name, chosen?))
                })
                .collect();
            if let Some(offer) = chosen.and_then(|chosen| offer(phrase, &chosen)) {
                offers.push(offer);
            }
        }
        offers
    }

    fn find(&mut self, kind: &str, wanted: &str) -> Vec<Dict> {
        let key = (kind.to_owned(), wanted.to_owned());
        if let Some(found) = self.found.get(&key) {
            return found.iter().filter_map(copied).collect();
        }
        let phrases = self.phrases;
        let found = phrases
            .things
            .get(kind)
            .filter(|findable| self.asks(&findable.bus))
            .and_then(|findable| self.heard(&findable.bus, self.look_up(findable, wanted)))
            .unwrap_or_default();
        let copy = found.iter().filter_map(copied).collect();
        self.found.insert(key, found);
        copy
    }

    fn look_up(&self, findable: &Findable, wanted: &str) -> Result<Vec<Dict>, Refusal> {
        let arguments = Dict::from([("text".to_owned(), plan::owned_text(wanted))]);
        self.client
            .find(&findable.bus, &findable.find, &arguments, "", MOST_FOUND)
            .map(|page| page.things)
            .and_then(|things| match (&findable.get, findable.accepts) {
                (Some(get), true) if !things.is_empty() => {
                    let references: Vec<String> = things
                        .iter()
                        .map(|thing| text(thing, "reference").to_owned())
                        .collect();
                    self.client
                        .get(&findable.bus, get, &references, &[], &Dict::new())
                }
                _ => Ok(things),
            })
    }
}

fn copied(dict: &Dict) -> Option<Dict> {
    crate::job::copied(dict).ok()
}

fn offer(phrase: &Served, chosen: &BTreeMap<&str, Chosen>) -> Option<Offer> {
    let mut arguments = Dict::new();
    for (name, word) in &phrase.fixed {
        let kind = &phrase.definition.parameter(name)?.kind;
        arguments.insert(name.clone(), kind.from_words(std::slice::from_ref(word))?);
    }
    for (name, chosen) in chosen {
        arguments.insert((*name).to_owned(), chosen.value.try_clone().ok()?);
    }
    let mut title = String::new();
    for token in &phrase.tokens {
        let (part, follows) = match token {
            Token::Literal(word) => (word.as_str(), false),
            Token::Placeholder(name) => {
                let chosen = chosen.get(name.as_str())?;
                (chosen.shown.as_str(), chosen.follows_thing)
            }
        };
        if !title.is_empty() {
            title.push_str(if follows { ": " } else { " " });
        }
        title.push_str(part);
    }
    let mut characters = title.chars();
    let title = characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())?;
    Some(Offer {
        title,
        detail: phrase.title.clone(),
        bus: phrase.bus.clone(),
        intent: phrase.intent.clone(),
        arguments,
    })
}

fn value(
    kind: &Kind,
    words: &[String],
    original: &[&str],
    free: bool,
    thing: &Dict,
    shared: bool,
) -> Option<Chosen> {
    let typed = words.join(" ");
    let one = match kind {
        Kind::List { items, .. } => items,
        kind => kind,
    };
    let (written, shown) = match one {
        Kind::Text { .. } if thing.contains_key(ACCEPTS) || thing.contains_key("minimum") => {
            return accepted(words, thing, shared);
        }
        Kind::Text { .. } if text(thing, "kind") == "time" && !shared => {
            let time = clock_time(&typed)?;
            return Some(Chosen {
                value: plan::owned_text(&time),
                shown: time,
                follows_thing: true,
            });
        }
        Kind::Text { .. } | Kind::RichText | Kind::Address if free => {
            (original.join(" "), original.join(" "))
        }
        Kind::Enum { words } => {
            let word = words.iter().find(|word| word.replace('-', " ") == typed)?;
            (word.clone(), typed)
        }
        Kind::Time => {
            let time = clock_time(&typed)?;
            (time.clone(), time)
        }
        Kind::DateTime => {
            let time = clock_time(&typed)?;
            (next_at(&time)?, time)
        }
        Kind::Duration => (duration(&typed)?, typed),
        Kind::Integer { .. } | Kind::Number | Kind::Date => (typed.clone(), typed),
        _ => return None,
    };
    Some(Chosen {
        value: kind.from_words(&[written])?,
        shown,
        follows_thing: false,
    })
}

fn clock_time(typed: &str) -> Option<String> {
    let (hour, minute) = typed.split_once([':', '.'])?;
    let time = jiff::civil::Time::new(hour.parse().ok()?, minute.parse().ok()?, 0, 0).ok()?;
    Some(time.strftime("%H:%M").to_string())
}

fn next_at(time: &str) -> Option<String> {
    let now = jiff::Zoned::now();
    let time: jiff::civil::Time = time.parse().ok()?;
    let today = now.with().time(time).build().ok()?;
    let next = match today > now {
        true => today,
        false => today.tomorrow().ok()?,
    };
    Some(next.to_string())
}

fn duration(typed: &str) -> Option<String> {
    let at = typed.find(|character: char| !character.is_ascii_digit())?;
    let (number, unit) = (typed[..at].parse::<u32>().ok()?, typed[at..].trim());
    let is = |names: String| names.split(',').any(|name| name.trim() == unit);
    if is(tr!("s, sec, secs, second, seconds")) {
        Some(format!("PT{number}S"))
    } else if is(tr!("m, min, mins, minute, minutes")) {
        Some(format!("PT{number}M"))
    } else if is(tr!("h, hr, hrs, hour, hours")) {
        Some(format!("PT{number}H"))
    } else {
        None
    }
}

fn list(thing: &Dict, name: &str) -> Vec<String> {
    match thing.get(name).map(|value| to_json(value)) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn accepted(words: &[String], thing: &Dict, shared: bool) -> Option<Chosen> {
    let number = |name: &str| thing.get(name).and_then(|value| to_json(value).as_i64());
    if let (Some(minimum), Some(maximum), false) = (number("minimum"), number("maximum"), shared) {
        let typed = words.concat();
        let wanted: i64 = typed.trim_end_matches('%').parse().ok()?;
        return (minimum..=maximum).contains(&wanted).then(|| Chosen {
            value: plan::owned_text(&wanted.to_string()),
            shown: wanted.to_string(),
            follows_thing: true,
        });
    }
    let accepts = list(thing, ACCEPTS);
    let labels = match list(thing, LABELS) {
        labels if labels.len() == accepts.len() => labels,
        _ => accepts.clone(),
    };
    let starts = |word: &String, label: &str| {
        label
            .to_lowercase()
            .split(|character: char| !character.is_alphanumeric())
            .any(|part| part.starts_with(word.as_str()))
    };
    let matches = |label: &str, word: &str| match shared {
        false => words
            .iter()
            .all(|typed| starts(typed, label) || typed == word),
        true => {
            let mine = words
                .iter()
                .filter(|typed| starts(typed, label) || *typed == word)
                .count();
            mine > 0 && (mine < words.len() || !SWITCH.contains(&word))
        }
    };
    accepts
        .iter()
        .zip(&labels)
        .find(|(word, label)| matches(label, word))
        .map(|(word, label)| Chosen {
            value: plan::owned_text(word),
            shown: label.clone(),
            follows_thing: true,
        })
}
