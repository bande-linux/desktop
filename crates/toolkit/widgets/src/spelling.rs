use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellRequest {
    Check(Vec<String>),
    Suggest(String),
    Learn(String),
    Correct(String),
    Rules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpellReply {
    Checked(Vec<(String, bool)>),
    Suggested(String, Vec<String>),
    Corrected(String, String),
    Rules(HashMap<String, String>, HashMap<String, String>),
    Failed(Vec<String>),
    Changed,
    Absent,
}

#[derive(Default)]
struct State {
    send: Option<Box<dyn Fn(SpellRequest)>>,
    verdicts: HashMap<String, bool>,
    asked: HashSet<String>,
    suggestions: HashMap<String, Vec<String>>,
    ignored: HashSet<String>,
    generation: u64,
    corrections: HashMap<String, String>,
    rules: HashMap<String, String>,
    replacements: HashMap<String, String>,
    rules_asked: bool,
    changed_back: HashMap<String, u8>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

pub fn install_spelling(send: impl Fn(SpellRequest) + 'static) {
    STATE.with_borrow_mut(|state| state.send = Some(Box::new(send)));
}

pub fn receive_spelling(reply: SpellReply) {
    STATE.with_borrow_mut(|state| match reply {
        SpellReply::Checked(results) => {
            state.generation += 1;
            for (word, right) in results {
                state.asked.remove(&word);
                state.verdicts.insert(word, right);
            }
        }
        SpellReply::Suggested(word, suggestions) => {
            state.suggestions.insert(word, suggestions);
        }
        SpellReply::Corrected(word, correction) => {
            state.corrections.insert(word, correction);
        }
        SpellReply::Rules(rules, replacements) => {
            state.rules = rules;
            state.replacements = replacements;
        }
        SpellReply::Failed(words) => {
            for word in words {
                state.asked.remove(&word);
            }
        }
        SpellReply::Changed => {
            state.generation += 1;
            state.verdicts.clear();
            state.asked.clear();
            state.suggestions.clear();
            state.corrections.clear();
            if state.rules_asked
                && let Some(send) = &state.send
            {
                send(SpellRequest::Rules);
            }
        }
        SpellReply::Absent => {
            state.send = None;
            state.generation += 1;
            state.verdicts.clear();
            state.asked.clear();
        }
    });
}

pub(crate) fn generation() -> u64 {
    STATE.with_borrow(|state| state.generation)
}

pub(crate) fn wrong_ranges(text: &str, skip: Option<usize>) -> Vec<Range<usize>> {
    STATE.with_borrow_mut(|state| {
        if state.send.is_none() {
            return Vec::new();
        }
        let mut wrong = Vec::new();
        let mut missing = Vec::new();
        for range in words(text) {
            if skip.is_some_and(|at| range.start <= at && at <= range.end) {
                continue;
            }
            let word = &text[range.clone()];
            if state.ignored.contains(word) {
                continue;
            }
            match state.verdicts.get(word) {
                Some(false) => wrong.push(range),
                Some(true) => {}
                None => {
                    if state.asked.insert(word.to_owned()) {
                        missing.push(word.to_owned());
                    }
                }
            }
        }
        if !missing.is_empty()
            && let Some(send) = &state.send
        {
            send(SpellRequest::Check(missing));
        }
        wrong
    })
}

pub(crate) fn suggestions(word: &str) -> Option<Vec<String>> {
    STATE.with_borrow_mut(|state| {
        let known = state.suggestions.get(word).cloned();
        if known.is_none()
            && let Some(send) = &state.send
        {
            send(SpellRequest::Suggest(word.to_owned()));
        }
        known
    })
}

pub(crate) fn ensure_rules() {
    STATE.with_borrow_mut(|state| {
        if !state.rules_asked
            && let Some(send) = &state.send
        {
            state.rules_asked = true;
            send(SpellRequest::Rules);
        }
    });
}

pub(crate) fn rule(name: &str) -> String {
    STATE.with_borrow(|state| state.rules.get(name).cloned().unwrap_or_default())
}

pub(crate) fn option(name: &str) -> bool {
    rule(&format!("{name}-on")) == "true"
}

pub(crate) fn replacement(word: &str) -> Option<String> {
    STATE.with_borrow(|state| state.replacements.get(word).cloned())
}

pub(crate) fn ask_correction(word: &str) {
    STATE.with_borrow_mut(|state| {
        state.corrections.remove(word);
        if let Some(send) = &state.send {
            send(SpellRequest::Correct(word.to_owned()));
        }
    });
}

pub(crate) fn correction(word: &str) -> Option<String> {
    STATE.with_borrow(|state| state.corrections.get(word).cloned())
}

pub(crate) fn changed_back(word: &str) {
    let twice = STATE.with_borrow_mut(|state| {
        let count = state.changed_back.entry(word.to_owned()).or_default();
        *count += 1;
        *count >= 2
    });
    if twice {
        learn(word);
    } else {
        ignore(word);
    }
}

pub(crate) fn ignore(word: &str) {
    STATE.with_borrow_mut(|state| {
        state.ignored.insert(word.to_owned());
        state.generation += 1;
    });
}

pub(crate) fn learn(word: &str) {
    STATE.with_borrow_mut(|state| {
        state.verdicts.insert(word.to_owned(), true);
        state.generation += 1;
        if let Some(send) = &state.send {
            send(SpellRequest::Learn(word.to_owned()));
        }
    });
}

fn is_inner_joiner(character: char) -> bool {
    matches!(character, '\'' | '\u{2019}' | '-')
}

fn eligible(word: &str) -> bool {
    let mut characters = word.chars();
    characters.next();
    !characters.any(char::is_uppercase)
}

pub(crate) fn links(text: &str) -> Vec<Range<usize>> {
    if rule("links-on") == "false" {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut offset = 0;
    for chunk in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += chunk.len();
        let body = chunk
            .trim_end()
            .trim_start_matches(['(', '<', '[', '"'])
            .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '>', ']', '"']);
        let is_web =
            body.starts_with("http://") || body.starts_with("https://") || body.starts_with("www.");
        let is_mail = body.split_once('@').is_some_and(|(name, host)| {
            !name.is_empty() && host.contains('.') && !host.ends_with('.')
        });
        if (is_web || is_mail)
            && let Some(at) = chunk.find(body)
        {
            found.push(start + at..start + at + body.len());
        }
    }
    found
}

pub(crate) fn link_target(link: &str) -> String {
    if link.contains("://") {
        link.to_owned()
    } else if link.starts_with("www.") {
        format!("https://{link}")
    } else {
        format!("mailto:{link}")
    }
}

pub(crate) fn words(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut offset = 0;
    for chunk in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += chunk.len();
        let body = chunk.trim_end();
        if body.contains("://") || body.contains('@') || body.starts_with("www.") {
            continue;
        }
        let mut word: Option<usize> = None;
        let mut has_digit = false;
        let characters: Vec<(usize, char)> = body.char_indices().collect();
        for (index, &(at, character)) in characters.iter().enumerate() {
            let joined = is_inner_joiner(character)
                && word.is_some()
                && characters
                    .get(index + 1)
                    .is_some_and(|(_, next)| next.is_alphabetic());
            if character.is_alphabetic() || character.is_numeric() || joined {
                has_digit |= character.is_numeric();
                word.get_or_insert(at);
            } else if let Some(begin) = word.take() {
                if !has_digit && eligible(&body[begin..at]) {
                    found.push(start + begin..start + at);
                }
                has_digit = false;
            }
        }
        if let Some(begin) = word
            && !has_digit
            && eligible(&body[begin..])
        {
            found.push(start + begin..start + body.len());
        }
    }
    found
}
