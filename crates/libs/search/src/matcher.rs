pub const EXACT: u32 = 1000;
pub const PREFIX: u32 = 900;
pub const WORD: u32 = 800;
pub const WORDS: u32 = 750;
pub const INITIALS: u32 = 700;
pub const SUBSTRING: u32 = 600;
pub const FUZZY: u32 = 400;
const LENGTH_PENALTY_LIMIT: u32 = 99;
const FUZZY_GAP_PENALTY: u32 = 12;
const FUZZY_FLOOR: u32 = 150;
const MIN_LOOSE_LENGTH: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    folded: String,
    words: Vec<String>,
}

pub fn fold(text: &str) -> String {
    text.to_lowercase()
}

fn is_word_start(previous: Option<char>) -> bool {
    previous.is_none_or(|character| !character.is_alphanumeric())
}

fn word_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    let mut previous = None;
    text.char_indices().filter_map(move |(index, character)| {
        let start = is_word_start(previous) && character.is_alphanumeric();
        previous = Some(character);
        start.then_some(index)
    })
}

fn penalty(amount: usize) -> u32 {
    u32::try_from(amount)
        .unwrap_or(u32::MAX)
        .min(LENGTH_PENALTY_LIMIT)
}

impl Query {
    pub fn new(text: &str) -> Option<Self> {
        let folded = fold(text.trim());
        if folded.is_empty() {
            return None;
        }
        let words = folded.split_whitespace().map(str::to_owned).collect();
        Some(Self { folded, words })
    }

    pub fn text(&self) -> &str {
        &self.folded
    }

    fn feasible(&self, folded: &str) -> bool {
        let mut remaining = folded.chars();
        let ordered = self
            .folded
            .chars()
            .filter(|character| !character.is_whitespace())
            .all(|wanted| remaining.any(|character| character == wanted));
        ordered
            || (self.words.len() > 1
                && self.words.iter().all(|word| folded.contains(word.as_str())))
    }

    pub fn score(&self, folded: &str) -> Option<u32> {
        if !self.feasible(folded) {
            return None;
        }
        let query = self.folded.as_str();
        if folded == query {
            return Some(EXACT);
        }
        let extra = folded.chars().count().saturating_sub(query.chars().count());
        if folded.starts_with(query) {
            return Some(PREFIX - penalty(extra));
        }
        if word_starts(folded).any(|start| folded[start..].starts_with(query)) {
            return Some(WORD - penalty(extra));
        }
        if self.words.len() > 1
            && self.words.iter().all(|word| {
                word_starts(folded).any(|start| folded[start..].starts_with(word.as_str()))
            })
        {
            return Some(WORDS - penalty(extra));
        }
        if query.chars().count() > 1 && !query.contains(char::is_whitespace) {
            let initials: String = word_starts(folded)
                .filter_map(|start| folded[start..].chars().next())
                .collect();
            if initials.starts_with(query) {
                return Some(INITIALS - penalty(extra));
            }
        }
        if query.chars().count() < MIN_LOOSE_LENGTH {
            return None;
        }
        if let Some(position) = folded.find(query) {
            return Some(SUBSTRING - penalty(folded[..position].chars().count()));
        }
        self.fuzzy(folded)
    }

    pub fn matched(&self, folded: &str) -> Vec<usize> {
        if self.score(folded).is_none() {
            return Vec::new();
        }
        let query = self.folded.as_str();
        let span = |byte: usize, text: &str| {
            let start = folded[..byte].chars().count();
            start..start + text.chars().count()
        };
        let word_start =
            |text: &str| word_starts(folded).find(|start| folded[*start..].starts_with(text));
        if folded.starts_with(query) {
            return span(0, query).collect();
        }
        if let Some(start) = word_start(query) {
            return span(start, query).collect();
        }
        if self.words.len() > 1 && self.words.iter().all(|word| word_start(word).is_some()) {
            let mut positions: Vec<usize> = self
                .words
                .iter()
                .filter_map(|word| word_start(word).map(|start| span(start, word)))
                .flatten()
                .collect();
            positions.sort_unstable();
            positions.dedup();
            return positions;
        }
        let initials: String = word_starts(folded)
            .filter_map(|start| folded[start..].chars().next())
            .collect();
        if query.chars().count() > 1
            && !query.contains(char::is_whitespace)
            && initials.starts_with(query)
        {
            return word_starts(folded)
                .take(query.chars().count())
                .map(|start| folded[..start].chars().count())
                .collect();
        }
        if let Some(position) = folded.find(query) {
            return span(position, query).collect();
        }
        self.fuzzy_positions(folded)
    }

    fn fuzzy_positions(&self, folded: &str) -> Vec<usize> {
        let mut wanted = self
            .folded
            .chars()
            .filter(|c| !c.is_whitespace())
            .peekable();
        let mut positions = Vec::new();
        let mut previous = None;
        for (index, character) in folded.chars().enumerate() {
            let Some(&next) = wanted.peek() else {
                break;
            };
            if character == next && (!positions.is_empty() || is_word_start(previous)) {
                wanted.next();
                positions.push(index);
            }
            previous = Some(character);
        }
        positions
    }

    fn fuzzy(&self, folded: &str) -> Option<u32> {
        let mut wanted = self
            .folded
            .chars()
            .filter(|c| !c.is_whitespace())
            .peekable();
        wanted.peek()?;
        let mut score = FUZZY;
        let mut gap = 0usize;
        let mut previous = None;
        let mut started = false;
        for character in folded.chars() {
            let Some(&next) = wanted.peek() else {
                break;
            };
            if character == next && (started || is_word_start(previous)) {
                wanted.next();
                if started && !is_word_start(previous) {
                    score = score.saturating_sub(penalty(gap) * FUZZY_GAP_PENALTY);
                }
                started = true;
                gap = 0;
            } else if started {
                gap += 1;
            }
            previous = Some(character);
        }
        if wanted.peek().is_some() {
            return None;
        }
        (score >= FUZZY_FLOOR).then_some(score)
    }
}
