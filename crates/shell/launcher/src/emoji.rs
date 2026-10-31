use std::collections::HashSet;
use std::sync::LazyLock;

use bande_search::{Query, WORDS, fold};

const DATA: &str = include_str!("../data/emoji.tsv");
const SELECTOR: char = '\u{FE0F}';
pub const TONES: [char; 5] = [
    '\u{1F3FB}',
    '\u{1F3FC}',
    '\u{1F3FD}',
    '\u{1F3FE}',
    '\u{1F3FF}',
];
const KEYWORD_PENALTY: u32 = 150;

pub struct Emoji {
    pub character: &'static str,
    pub name: &'static str,
    pub group: usize,
    folded: String,
    keywords: Vec<String>,
}

pub struct EmojiSet {
    pub groups: Vec<&'static str>,
    pub emoji: Vec<Emoji>,
    toned: HashSet<&'static str>,
}

pub static EMOJI: LazyLock<EmojiSet> = LazyLock::new(|| EmojiSet::parse(DATA));

impl EmojiSet {
    fn parse(data: &'static str) -> Self {
        let mut set = Self {
            groups: Vec::new(),
            emoji: Vec::new(),
            toned: HashSet::new(),
        };
        for line in data.lines() {
            let mut fields = line.split('\t');
            let (Some(character), Some(tones), Some(name), Some(keywords)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                if !line.is_empty() {
                    set.groups.push(line);
                }
                continue;
            };
            if !tones.is_empty() {
                set.toned.insert(character);
            }
            set.emoji.push(Emoji {
                character,
                name,
                group: set.groups.len().saturating_sub(1),
                folded: fold(name),
                keywords: keywords
                    .split('|')
                    .filter(|keyword| !keyword.is_empty())
                    .map(fold)
                    .collect(),
            });
        }
        set
    }

    pub fn find(&self, character: &str) -> Option<&Emoji> {
        self.emoji.iter().find(|emoji| emoji.character == character)
    }

    pub fn has_tones(&self, character: &str) -> bool {
        self.toned.contains(character)
    }

    pub fn search(&self, query: &Query) -> Vec<(&Emoji, u32, Vec<usize>)> {
        let mut found: Vec<(&Emoji, u32, Vec<usize>)> = self
            .emoji
            .iter()
            .filter_map(|emoji| {
                let named = query.score(&emoji.folded);
                let described = emoji
                    .keywords
                    .iter()
                    .filter_map(|keyword| query.score(keyword))
                    .filter(|score| *score >= WORDS)
                    .max()
                    .map(|score| score - KEYWORD_PENALTY);
                let score = named.max(described)?;
                let matched = if named == Some(score) {
                    query.matched(&emoji.folded)
                } else {
                    Vec::new()
                };
                Some((emoji, score, matched))
            })
            .collect();
        found.sort_by_key(|(_, score, _)| std::cmp::Reverse(*score));
        found
    }

    pub fn with_tone(&self, character: &str, tone: Option<usize>) -> String {
        let Some(modifier) = tone
            .and_then(|tone| TONES.get(tone))
            .filter(|_| self.has_tones(character))
        else {
            return character.to_owned();
        };
        let mut characters = character.chars();
        let mut toned: String = characters.next().into_iter().collect();
        toned.push(*modifier);
        let rest = characters.as_str();
        toned.push_str(rest.strip_prefix(SELECTOR).unwrap_or(rest));
        toned
    }
}

#[cfg(test)]
mod tests {
    use super::EMOJI;

    #[test]
    fn the_compiled_set_has_every_group_and_skin_tones_follow_the_first_character() {
        assert_eq!(EMOJI.groups.len(), 10);
        assert!(EMOJI.emoji.len() > 1800);
        assert_eq!(EMOJI.with_tone("👋", Some(2)), "👋\u{1F3FD}");
        assert_eq!(
            EMOJI.with_tone("🏃\u{200D}♀\u{FE0F}", Some(0)),
            "🏃\u{1F3FB}\u{200D}♀\u{FE0F}"
        );
        assert_eq!(EMOJI.with_tone("😀", Some(2)), "😀");
        assert_eq!(EMOJI.with_tone("👋", None), "👋");
    }
}
