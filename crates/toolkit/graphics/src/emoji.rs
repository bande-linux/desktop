use std::ops::Range;

use bande_design::{Color, Typeface};
use parley::{RangedBuilder, StyleProperty};
use parley_data::Properties;

use crate::text::families;

const AS_TEXT: char = '\u{fe0e}';
const AS_EMOJI: char = '\u{fe0f}';
const KEYCAP: char = '\u{20e3}';
const JOINER: char = '\u{200d}';
const TAGS: std::ops::RangeInclusive<char> = '\u{e0020}'..='\u{e007f}';
const FIRST_EMOJI_BYTE: u8 = 0xe2;

struct Scanner<'a> {
    text: &'a str,
    at: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn take(&mut self, wanted: impl FnOnce(char) -> bool) -> Option<char> {
        let next = self.peek().filter(|next| wanted(*next))?;
        self.at += next.len_utf8();
        Some(next)
    }

    /// Reads one emoji sequence and tells whether it is shown as an emoji rather than as text.
    fn sequence(&mut self) -> Option<bool> {
        let first = self.peek()?;
        let properties = Properties::get(first);
        if properties.is_region_indicator() {
            self.take(|_| true);
            let pair = self.take(|next| Properties::get(next).is_region_indicator());
            return Some(pair.is_some());
        }
        if !properties.is_emoji() {
            return None;
        }
        let mut shown = self.element(properties.is_emoji_presentation());
        loop {
            let before = self.at;
            let joined = self.take(|next| next == JOINER).and_then(|_| self.peek());
            match joined.map(Properties::get) {
                Some(next) if next.is_emoji() => {
                    self.element(true);
                    shown = true;
                }
                _ => {
                    self.at = before;
                    return Some(shown);
                }
            }
        }
    }

    fn element(&mut self, presentation: bool) -> bool {
        let first = self.take(|_| true);
        let modified = self
            .take(|next| Properties::get(next).is_emoji_modifier())
            .is_some();
        let selector = self.take(|next| matches!(next, AS_TEXT | AS_EMOJI));
        let keycap = self.take(|next| next == KEYCAP).is_some();
        let mut tagged = false;
        while self.take(|next| TAGS.contains(&next)).is_some() {
            tagged = true;
        }
        match selector {
            Some(AS_TEXT) => false,
            Some(_) => keycap || first.is_some_and(|first| !first.is_ascii()),
            None => presentation || modified || tagged,
        }
    }
}

pub(crate) fn ranges(text: &str) -> Vec<Range<usize>> {
    let mut found: Vec<Range<usize>> = Vec::new();
    if text.bytes().all(|byte| byte < FIRST_EMOJI_BYTE) {
        return found;
    }
    let mut scanner = Scanner { text, at: 0 };
    while scanner.at < text.len() {
        let start = scanner.at;
        match scanner.sequence() {
            Some(true) => match found.last_mut() {
                Some(last) if last.end == start => last.end = scanner.at,
                _ => found.push(start..scanner.at),
            },
            Some(false) => {}
            None => {
                scanner.take(|_| true);
            }
        }
    }
    found
}

/// Sets the emoji of `text` in the colour emoji font. Push it after the other font families.
pub fn push_emoji(builder: &mut RangedBuilder<'_, Color>, text: &str) {
    for range in ranges(text) {
        builder.push(StyleProperty::FontFamily(families(Typeface::Emoji)), range);
    }
}

#[cfg(test)]
mod tests {
    use bande_design::{Color, Typeface};

    use crate::outline::GlyphOutlines;
    use crate::text::{TextAlign, TextStyle, TextSystem};

    #[test]
    fn emoji_are_in_colour_and_text_stays_text() {
        let mut text = TextSystem::new();
        if !text.has_family(Typeface::Emoji.family()) {
            eprintln!("skipped, the colour emoji font is not installed");
            return;
        }
        let style = TextStyle::new(13.0, Color::BLACK);
        for style in [style, style.monospace()] {
            let mut runs = |shown: &str| {
                let paragraph = text.paragraph(shown, style, None, TextAlign::Start);
                let mut outlines = GlyphOutlines::default();
                paragraph
                    .glyph_runs()
                    .iter()
                    .map(|run| (outlines.is_color_font(run), run.glyphs.len()))
                    .collect::<Vec<_>>()
            };
            for emoji in [
                "😀",
                "❤️",
                "☺️",
                "⚡",
                "©️",
                "1️⃣",
                "#️⃣",
                "🇸🇪",
                "👍🏽",
                "☝🏽",
                "👩‍👩‍👧",
                "❤️‍🔥",
                "🏳️‍🌈",
                "🏴󠁧󠁢󠁳󠁣󠁴󠁿",
            ] {
                assert_eq!(runs(emoji), [(true, 1)], "{emoji}");
            }
            for plain in ["1234#*", "© ™", "✔", "❤", "☺", "⚡\u{fe0e}", "é – … 漢字"]
            {
                assert!(runs(plain).iter().all(|(colour, _)| !colour), "{plain}");
            }
            let mixed = runs("a⚡b");
            assert_eq!(
                mixed.iter().map(|run| run.0).collect::<Vec<_>>(),
                [false, true, false]
            );
        }
    }
}
