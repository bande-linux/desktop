use std::ops::Range;
use std::sync::Arc;

use bande_graphics::Color;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextAttributes {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub family: Option<Arc<str>>,
    pub size: Option<f32>,
    pub color: Option<Color>,
    pub link: Option<Arc<str>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Alignment {
    #[default]
    Start,
    Center,
    End,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMarker {
    Bullet,
    Number,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParagraphAttributes {
    pub alignment: Alignment,
    pub list: Option<ListMarker>,
    pub level: u8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichText {
    pub text: String,
    pub runs: Vec<(Range<usize>, TextAttributes)>,
    pub paragraphs: Vec<ParagraphAttributes>,
}

pub(super) type Runs = Vec<(usize, TextAttributes)>;

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Fragment {
    pub(super) text: String,
    pub(super) runs: Runs,
    pub(super) breaks: Vec<ParagraphAttributes>,
}

pub(super) fn normalize(runs: Runs) -> Runs {
    let mut merged: Runs = Vec::with_capacity(runs.len());
    for (length, attributes) in runs {
        if length == 0 {
            continue;
        }
        match merged.last_mut() {
            Some((last, previous)) if *previous == attributes => *last += length,
            _ => merged.push((length, attributes)),
        }
    }
    match merged
        .iter()
        .all(|(_, attributes)| *attributes == TextAttributes::default())
    {
        true => Vec::new(),
        false => merged,
    }
}

pub(super) fn cut(runs: &Runs, range: Range<usize>) -> Runs {
    let mut cut = Vec::new();
    let mut at = 0;
    for (length, attributes) in runs {
        let (start, end) = (at.max(range.start), (at + length).min(range.end));
        if start < end {
            cut.push((end - start, attributes.clone()));
        }
        at += length;
    }
    cut
}

pub(super) fn joined(first: Runs, first_length: usize, second: Runs, second_length: usize) -> Runs {
    if first.is_empty() && second.is_empty() {
        return Vec::new();
    }
    let filled = |runs: Runs, length: usize| match runs.is_empty() {
        true => vec![(length, TextAttributes::default())],
        false => runs,
    };
    let mut runs = filled(first, first_length);
    runs.extend(filled(second, second_length));
    normalize(runs)
}

pub(super) fn attributes_at(runs: &Runs, offset: usize) -> TextAttributes {
    let mut at = 0;
    for (length, attributes) in runs {
        at += length;
        if offset < at {
            return attributes.clone();
        }
    }
    TextAttributes::default()
}

impl Fragment {
    pub(super) fn plain(
        text: &str,
        attributes: TextAttributes,
        paragraph: ParagraphAttributes,
    ) -> Self {
        Self {
            text: text.to_owned(),
            runs: normalize(vec![(text.len(), attributes)]),
            breaks: vec![paragraph; text.matches('\n').count()],
        }
    }

    pub(super) fn append(&mut self, other: Fragment) {
        let length = self.text.len();
        self.runs = joined(
            std::mem::take(&mut self.runs),
            length,
            other.runs,
            other.text.len(),
        );
        self.text.push_str(&other.text);
        self.breaks.extend(other.breaks);
    }

    pub(super) fn lines(&self) -> impl Iterator<Item = (&str, Runs)> + '_ {
        let mut start = 0;
        self.text.split('\n').map(move |line| {
            let range = start..start + line.len();
            start = range.end + 1;
            (line, normalize(cut(&self.runs, range)))
        })
    }
}

impl RichText {
    pub(super) fn line_runs(&self) -> Vec<Runs> {
        let mut lines = Vec::new();
        let mut start = 0;
        for line in self.text.split('\n') {
            let range = start..start + line.len();
            let runs = self
                .runs
                .iter()
                .filter_map(|(run, attributes)| {
                    let (from, to) = (run.start.max(range.start), run.end.min(range.end));
                    (from < to).then(|| (from - range.start..to - range.start, attributes))
                })
                .scan(0, |at, (run, attributes)| {
                    let gap = (run.start - *at, TextAttributes::default());
                    *at = run.end;
                    Some([gap, (run.len(), attributes.clone())])
                })
                .flatten()
                .collect::<Runs>();
            let covered: usize = runs.iter().map(|(length, _)| length).sum();
            let mut runs = runs;
            runs.push((
                line.len().saturating_sub(covered),
                TextAttributes::default(),
            ));
            lines.push(normalize(runs));
            start = range.end + 1;
        }
        lines
    }
}
