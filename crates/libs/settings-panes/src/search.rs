use std::ops::Range;

use crate::registry::{Pane, PaneId, Setting, SettingId};

const LABEL: u32 = 100;
const PANE_NAME: u32 = 90;
const KEYWORD: u32 = 60;
const CHOICE: u32 = 40;
const SECTION: u32 = 30;
const PANE_KEYWORD: u32 = 30;
const WORD_START: u32 = 20;
const LABEL_START: u32 = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingHit {
    pub setting: SettingId,
    pub score: u32,
    pub label: Vec<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneHit {
    pub pane: PaneId,
    pub score: u32,
    pub name: Vec<Range<usize>>,
    pub settings: Vec<SettingHit>,
}

impl PaneHit {
    pub fn best(&self) -> Option<&SettingHit> {
        self.settings.iter().max_by_key(|hit| hit.score)
    }

    pub fn setting(&self, id: SettingId) -> Option<&SettingHit> {
        self.settings.iter().find(|hit| hit.setting == id)
    }
}

fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

fn lowered(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    (lower.len() == text.len()).then_some(lower)
}

fn find(field: &str, word: &str) -> Option<(Range<usize>, bool)> {
    let field = lowered(field)?;
    let mut fallback = None;
    for (at, _) in field.match_indices(word) {
        let starts_word = field[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric());
        let range = at..at + word.len();
        if starts_word {
            return Some((range, true));
        }
        fallback.get_or_insert((range, false));
    }
    fallback
}

fn score(field: &str, word: &str, weight: u32) -> Option<u32> {
    find(field, word).map(|(_, starts_word)| weight + if starts_word { WORD_START } else { 0 })
}

fn best<'a>(fields: impl IntoIterator<Item = (&'a str, u32)>, word: &str) -> Option<u32> {
    fields
        .into_iter()
        .filter_map(|(field, weight)| score(field, word, weight))
        .max()
}

fn pane_fields(pane: &Pane) -> impl Iterator<Item = (&str, u32)> {
    std::iter::once((pane.name.as_str(), PANE_NAME)).chain(
        pane.keywords
            .iter()
            .map(|keyword| (keyword.as_str(), PANE_KEYWORD)),
    )
}

fn setting_fields<'a>(
    section: Option<&'a str>,
    setting: &'a Setting,
) -> impl Iterator<Item = (&'a str, u32)> {
    std::iter::once((setting.label.as_str(), LABEL))
        .chain(
            setting
                .keywords
                .iter()
                .map(|keyword| (keyword.as_str(), KEYWORD)),
        )
        .chain(
            setting
                .kind
                .choices()
                .iter()
                .map(|choice| (choice.label.as_str(), CHOICE)),
        )
        .chain(section.map(|title| (title, SECTION)))
}

fn ranges(field: &str, words: &[String]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = words
        .iter()
        .filter_map(|word| Some(find(field, word)?.0))
        .collect();
    ranges.sort_by_key(|range| range.start);
    ranges.dedup_by(|later, earlier| {
        if later.start <= earlier.end {
            earlier.end = earlier.end.max(later.end);
            true
        } else {
            false
        }
    });
    ranges
}

fn setting_hit(
    pane: &Pane,
    section: Option<&str>,
    setting: &Setting,
    words: &[String],
) -> Option<SettingHit> {
    let mut total = 0;
    let mut own = false;
    for word in words {
        let mine = best(setting_fields(section, setting), word);
        own |= mine.is_some();
        total += mine.or_else(|| best(pane_fields(pane), word))?;
    }
    if !own {
        return None;
    }
    let query = words.join(" ");
    if lowered(&setting.label).is_some_and(|label| label.starts_with(&query)) {
        total += LABEL_START;
    }
    Some(SettingHit {
        setting: setting.id,
        score: total,
        label: ranges(&setting.label, words),
    })
}

pub fn search(panes: &[Pane], query: &str) -> Vec<PaneHit> {
    let words = words(query);
    if words.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<PaneHit> = panes
        .iter()
        .filter_map(|pane| {
            let settings: Vec<SettingHit> = pane
                .sections
                .iter()
                .flat_map(|section| {
                    section.settings.iter().filter_map(|setting| {
                        setting_hit(pane, section.title.as_deref(), setting, &words)
                    })
                })
                .collect();
            let own = words
                .iter()
                .map(|word| best(pane_fields(pane), word))
                .sum::<Option<u32>>();
            let score = settings.iter().map(|hit| hit.score).chain(own).max()?;
            Some(PaneHit {
                pane: pane.id,
                score,
                name: ranges(&pane.name, &words),
                settings,
            })
        })
        .collect();
    hits.sort_by_key(|hit| std::cmp::Reverse(hit.score));
    hits
}
