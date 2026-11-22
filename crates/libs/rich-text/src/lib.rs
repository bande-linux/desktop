use std::sync::Arc;

use bande_toolkit::Color;
use bande_toolkit::ui::{ParagraphAttributes, RichText, TextAttributes};

pub mod html;

pub const MONOSPACE: &str = "JetBrains Mono";
pub const HEADINGS: [f32; 6] = [26.0, 22.0, 18.0, 16.0, 15.0, 14.0];
pub const LINK: Color = Color::hex(0x0066D9);
pub const MAX_NESTING: usize = 64;

pub fn heading(level: usize) -> TextAttributes {
    TextAttributes {
        bold: true,
        size: HEADINGS.get(level.saturating_sub(1)).copied(),
        ..TextAttributes::default()
    }
}

pub fn heading_level(attributes: &TextAttributes) -> Option<usize> {
    let size = attributes.size?;
    HEADINGS
        .iter()
        .position(|heading| (heading - size).abs() < 0.5)
        .filter(|_| attributes.bold)
        .map(|index| index + 1)
}

pub const SERIF: &str = "Noto Serif";

pub fn family(name: &str) -> Option<Arc<str>> {
    let lower = name.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| lower.contains(word));
    match () {
        _ if has(&[
            "mono", "courier", "menlo", "monaco", "consolas", "code", "fixed",
        ]) =>
        {
            Some(Arc::from(MONOSPACE))
        }
        _ if has(&[
            "sans",
            "helvetica",
            "arial",
            "calibri",
            "inter",
            "verdana",
            "segoe",
            "tahoma",
            "lucida grande",
            "aptos",
            "carlito",
            "liberation sans",
        ]) =>
        {
            None
        }
        _ if has(&[
            "serif",
            "times",
            "georgia",
            "garamond",
            "cambria",
            "palatino",
            "baskerville",
            "caladea",
        ]) =>
        {
            Some(Arc::from(SERIF))
        }
        _ if name.trim().is_empty() => None,
        _ => Some(Arc::from(name.trim())),
    }
}

pub fn code() -> TextAttributes {
    TextAttributes {
        family: Some(Arc::from(MONOSPACE)),
        ..TextAttributes::default()
    }
}

pub fn is_code(attributes: &TextAttributes) -> bool {
    attributes.family.as_deref() == Some(MONOSPACE)
}

pub fn link(url: &str) -> TextAttributes {
    TextAttributes {
        underline: true,
        color: Some(LINK),
        link: Some(Arc::from(url)),
        ..TextAttributes::default()
    }
}

#[derive(Default)]
pub struct Builder {
    rich: RichText,
    paragraph: ParagraphAttributes,
}

impl Builder {
    pub fn text(&mut self, text: &str, attributes: &TextAttributes) {
        let text = text.replace(['\n', '\r'], " ");
        if text.is_empty() {
            return;
        }
        let start = self.rich.text.len();
        self.rich.text.push_str(&text);
        if *attributes != TextAttributes::default() {
            match self.rich.runs.last_mut() {
                Some((range, last)) if range.end == start && last == attributes => {
                    range.end = self.rich.text.len();
                }
                _ => self
                    .rich
                    .runs
                    .push((start..self.rich.text.len(), attributes.clone())),
            }
        }
    }

    pub fn paragraph(&mut self) -> &mut ParagraphAttributes {
        &mut self.paragraph
    }

    pub fn is_empty_paragraph(&self) -> bool {
        self.rich.text.is_empty() || self.rich.text.ends_with('\n')
    }

    pub fn is_empty(&self) -> bool {
        self.rich.text.is_empty()
    }

    pub fn line_break(&mut self) {
        self.end_paragraph(self.paragraph);
    }

    pub fn end_paragraph(&mut self, next: ParagraphAttributes) {
        self.rich.paragraphs.push(self.paragraph);
        self.rich.text.push('\n');
        self.paragraph = next;
    }

    pub fn finish(mut self) -> RichText {
        while self.rich.text.ends_with("\n\n") {
            self.rich.text.pop();
            self.rich.paragraphs.pop();
        }
        if self.rich.text.ends_with('\n') {
            self.rich.text.pop();
        } else {
            self.rich.paragraphs.push(self.paragraph);
        }
        if self.rich.paragraphs.is_empty() {
            self.rich.paragraphs.push(self.paragraph);
        }
        self.rich
    }
}

pub struct Paragraph<'a> {
    pub attributes: ParagraphAttributes,
    pub spans: Vec<(&'a str, TextAttributes)>,
}

impl Paragraph<'_> {
    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|(text, _)| text.is_empty())
    }

    pub fn first(&self) -> TextAttributes {
        self.spans
            .iter()
            .find(|(text, _)| !text.trim().is_empty())
            .map(|(_, attributes)| attributes.clone())
            .unwrap_or_default()
    }
}

pub fn paragraphs(rich: &RichText) -> Vec<Paragraph<'_>> {
    let mut paragraphs = Vec::new();
    let mut runs = rich.runs.iter().peekable();
    let mut start = 0;
    for (index, line) in rich.text.split('\n').enumerate() {
        let end = start + line.len();
        let mut spans = Vec::new();
        let mut at = start;
        while at < end {
            while runs.peek().is_some_and(|(range, _)| range.end <= at) {
                runs.next();
            }
            let (until, attributes) = match runs.peek() {
                Some((range, attributes)) if range.start <= at => {
                    (range.end.min(end), (*attributes).clone())
                }
                Some((range, _)) => (range.start.min(end), TextAttributes::default()),
                None => (end, TextAttributes::default()),
            };
            spans.push((&rich.text[at..until], attributes));
            at = until;
        }
        paragraphs.push(Paragraph {
            attributes: rich.paragraphs.get(index).copied().unwrap_or_default(),
            spans,
        });
        start = end + 1;
    }
    paragraphs
}
