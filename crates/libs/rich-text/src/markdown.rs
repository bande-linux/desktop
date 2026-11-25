use std::borrow::Cow;
use std::cmp::Reverse;

use crate::{
    Builder, ListMarker, Paragraph, ParagraphAttributes, RichText, TextAttributes, code, heading,
    heading_level, is_code, link, paragraphs,
};

const SEARCH_LIMIT: usize = 4096;
const MAX_LEVEL: usize = 8;
const MAX_BRACKETS: usize = 32;
const MAX_NUMBER_DIGITS: usize = 9;
const MIN_FENCE: usize = 3;
const BYTE_ORDER_MARK: char = '\u{feff}';
const FOREIGN_AUTOLINKS: [&str; 2] = ["ftp", "xmpp"];
const MARKERS: [&str; 3] = ["**", "*", "~~"];

#[derive(Clone, Copy, PartialEq)]
enum Dialect {
    Authored,
    Sanitised,
}

struct Reader {
    builder: Builder,
    open: bool,
    gap: bool,
    listing: bool,
}

impl Reader {
    fn close(&mut self) {
        if self.open {
            self.builder.end_paragraph(ParagraphAttributes::default());
            self.open = false;
        }
    }

    fn start(&mut self, attributes: ParagraphAttributes, listed: bool) {
        self.close();
        if !self.builder.is_empty() && (self.gap || self.listing != listed) {
            self.builder.end_paragraph(ParagraphAttributes::default());
        }
        self.gap = false;
        self.listing = listed;
        *self.builder.paragraph() = attributes;
        self.open = true;
    }
}

fn list_item(line: &str) -> Option<(usize, ListMarker, &str)> {
    let content = line.trim_start_matches(' ');
    let indent = line.len() - content.len();
    let level = (indent / 2).min(MAX_LEVEL);
    for bullet in ["- ", "* ", "+ "] {
        if let Some(rest) = content.strip_prefix(bullet) {
            return Some((level, ListMarker::Bullet, rest));
        }
    }
    let digits = content.chars().take_while(char::is_ascii_digit).count();
    let rest = &content[digits..];
    match (
        digits,
        rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")),
    ) {
        (1..=MAX_NUMBER_DIGITS, Some(rest)) => Some((level, ListMarker::Number, rest)),
        _ => None,
    }
}

const DELIMITERS: [&str; 6] = ["`", "**", "__", "~~", "*", "_"];

struct Closings(Vec<Vec<usize>>);

impl Closings {
    fn new(text: &str) -> Self {
        Self(
            DELIMITERS
                .iter()
                .map(|delimiter| text.match_indices(delimiter).map(|(at, _)| at).collect())
                .collect(),
        )
    }

    fn after(&self, from: usize, delimiter: &str) -> Option<usize> {
        let index = DELIMITERS.iter().position(|known| *known == delimiter)?;
        let found = &self.0[index];
        found
            .get(found.partition_point(|at| *at < from))
            .copied()
            .filter(|at| *at - from <= SEARCH_LIMIT)
    }
}

fn label_end(text: &str, from: usize, dialect: Dialect) -> Option<usize> {
    let mut open_images = Vec::new();
    let mut previous = None;
    let mut characters = text[from..].char_indices();
    while let Some((at, character)) = characters.next() {
        if at > SEARCH_LIMIT {
            return None;
        }
        match character {
            '\\' => {
                characters.next();
            }
            '[' if open_images.len() == MAX_BRACKETS => return None,
            '[' => open_images.push(dialect == Dialect::Sanitised && previous == Some('!')),
            ']' => {
                let targeted = text[from + at + 1..].starts_with('(');
                match open_images.pop() {
                    None => return targeted.then_some(from + at),
                    Some(false) if targeted => return None,
                    Some(_) => {}
                }
            }
            _ => {}
        }
        previous = Some(character);
    }
    None
}

fn destination_end(text: &str, from: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quote = None;
    let mut characters = text[from..].char_indices();
    while let Some((at, character)) = characters.next() {
        if at > SEARCH_LIMIT {
            return None;
        }
        match (character, quote) {
            ('\\', _) => {
                characters.next();
            }
            ('<', None) if text[from..from + at].trim().is_empty() => quote = Some('>'),
            ('"', None) if text[from..from + at].ends_with(' ') => quote = Some('"'),
            ('<', Some('>')) => return None,
            (character, Some(end)) if character == end => quote = None,
            (_, Some(_)) => {}
            ('(', None) if depth == MAX_BRACKETS => return None,
            ('(', None) => depth += 1,
            (')', None) if depth == 0 => return Some(from + at),
            (')', None) => depth -= 1,
            _ => {}
        }
    }
    None
}

fn unescaped(destination: &str) -> String {
    let mut out = String::with_capacity(destination.len());
    let mut characters = destination.chars().peekable();
    while let Some(character) = characters.next() {
        let escaped = (character == '\\')
            .then(|| characters.next_if(char::is_ascii_punctuation))
            .flatten();
        out.push(escaped.unwrap_or(character));
    }
    out.replace("&amp;", "&")
}

fn target(destination: &str) -> Option<String> {
    let destination = match destination.strip_prefix('<') {
        Some(rest) => rest.split_once('>')?.0,
        None => destination.split_whitespace().next()?,
    };
    allowed(&unescaped(destination))
}

fn allowed(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    ["https://", "http://", "mailto:"]
        .iter()
        .any(|scheme| lower.len() > scheme.len() && lower.starts_with(scheme))
        .then(|| url.trim().to_owned())
}

fn percent_encoded(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for character in url.chars() {
        match character {
            '(' | ')' | '<' | '>' | '"' | '\\' | '`' => {
                out.push_str(&format!("%{:02X}", u32::from(character)));
            }
            character if character.is_control() || character.is_whitespace() => {
                for byte in character.encode_utf8(&mut [0; 4]).bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
            character => out.push(character),
        }
    }
    out
}

fn linked(url: &str, attributes: &TextAttributes) -> TextAttributes {
    TextAttributes {
        bold: attributes.bold,
        italic: attributes.italic,
        strikethrough: attributes.strikethrough,
        size: attributes.size,
        ..link(url)
    }
}

fn inline(builder: &mut Builder, text: &str, base: &TextAttributes, dialect: Dialect) {
    let closings = Closings::new(text);
    let mut attributes = base.clone();
    let mut toggled = [false; 3];
    let mut plain = String::new();
    let mut at = 0;
    let flush = |builder: &mut Builder, plain: &mut String, attributes: &TextAttributes| {
        builder.text(plain, attributes);
        plain.clear();
    };
    while let Some(character) = text[at..].chars().next() {
        let rest = &text[at..];
        if character == '\\'
            && let Some(escaped) = rest[1..].chars().next()
            && escaped.is_ascii_punctuation()
        {
            plain.push(escaped);
            at += 1 + escaped.len_utf8();
            continue;
        }
        if character == '`'
            && let Some(end) = closings.after(at + 1, "`")
        {
            flush(builder, &mut plain, &attributes);
            let code = TextAttributes {
                family: super::code().family,
                ..attributes.clone()
            };
            builder.text(&text[at + 1..end], &code);
            at = end + 1;
            continue;
        }
        let image = dialect == Dialect::Sanitised && rest.starts_with("![");
        let label = at + 1 + usize::from(image);
        if (character == '[' || image)
            && let Some(middle) = label_end(text, label, dialect)
            && let Some(end) = destination_end(text, middle + 2)
        {
            flush(builder, &mut plain, &attributes);
            let written = text[middle + 2..end].trim();
            let url = match dialect {
                Dialect::Authored => Some(written.to_owned()),
                Dialect::Sanitised => target(written).filter(|_| !image),
            };
            let shown = url.map_or_else(|| attributes.clone(), |url| linked(&url, &attributes));
            inline(builder, &text[label..middle], &shown, dialect);
            at = end + 1;
            continue;
        }
        if dialect == Dialect::Sanitised
            && character == '<'
            && let Some(length) = rest[1..].find(['<', '>', ' '])
            && rest[1 + length..].starts_with('>')
            && let Some(url) = allowed(&rest[1..1 + length])
        {
            flush(builder, &mut plain, &attributes);
            builder.text(&url, &linked(&url, &attributes));
            at += length + 2;
            continue;
        }
        let toggles = [("**", 0), ("__", 0), ("~~", 2), ("*", 1), ("_", 1)];
        let longest = toggles
            .iter()
            .find(|(delimiter, _)| rest.starts_with(delimiter));
        let toggle = longest.filter(|(delimiter, kind)| {
            let active = toggled[*kind];
            let before = text[..at].chars().next_back();
            let allowed = *delimiter != "_" || active || !before.is_some_and(char::is_alphanumeric);
            let after = rest[delimiter.len()..].chars().next();
            let flanking = match active {
                true => before.is_some_and(|character| !character.is_whitespace()),
                false => after.is_some_and(|character| !character.is_whitespace()),
            };
            allowed
                && flanking
                && (active || closings.after(at + delimiter.len(), delimiter).is_some())
        });
        if let Some((delimiter, kind)) = toggle {
            flush(builder, &mut plain, &attributes);
            toggled[*kind] = !toggled[*kind];
            attributes.bold = base.bold || toggled[0];
            attributes.italic = base.italic || toggled[1];
            attributes.strikethrough = base.strikethrough || toggled[2];
            at += delimiter.len();
            continue;
        }
        let literal = longest.map_or(character.len_utf8(), |(delimiter, _)| delimiter.len());
        plain.push_str(&rest[..literal]);
        at += literal;
    }
    flush(builder, &mut plain, &attributes);
}

fn fence(content: &str) -> Option<(char, usize)> {
    let character = content
        .chars()
        .next()
        .filter(|first| "`~".contains(*first))?;
    let rest = content.trim_start_matches(character);
    let length = content.len() - rest.len();
    (length >= MIN_FENCE && !(character == '`' && rest.contains('`')))
        .then_some((character, length))
}

pub fn read(text: &str) -> RichText {
    parse(text, Dialect::Authored)
}

fn printable(character: char) -> char {
    match character {
        '\t' => character,
        BYTE_ORDER_MARK | '\u{2028}' | '\u{2029}' => ' ',
        _ if character.is_control() => ' ',
        _ => character,
    }
}

fn without_metadata(text: &str) -> &str {
    let Some((first, rest)) = text.split_once('\n') else {
        return text;
    };
    if first.trim_end() != "---"
        || rest
            .lines()
            .next()
            .is_none_or(|line| line.trim().is_empty())
    {
        return text;
    }
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        at += line.len();
        if matches!(line.trim_end(), "---" | "...") {
            return &rest[at..];
        }
    }
    text
}

/// Reads untrusted Markdown: images become their description, raw HTML becomes text, a metadata
/// block is left out and only links to `https`, `http` and `mailto` keep their target.
pub fn read_sanitised(text: &str) -> RichText {
    let text: String = text
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{2029}'], "\n")
        .chars()
        .filter(|character| *character == '\n' || printable(*character) == *character)
        .collect();
    parse(without_metadata(&text), Dialect::Sanitised)
}

/// Reads untrusted Markdown with [`read_sanitised`] and writes it again.
pub fn sanitise(text: &str) -> String {
    write_sanitised(&read_sanitised(text))
}

fn parse(text: &str, dialect: Dialect) -> RichText {
    let mut reader = Reader {
        builder: Builder::default(),
        open: false,
        gap: false,
        listing: false,
    };
    let mut open_fence = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some((character, length)) = open_fence {
            let content = trimmed.trim_start();
            if content.len() >= length && content.trim_matches(character).is_empty() {
                open_fence = None;
                reader.gap = true;
                continue;
            }
            reader.start(ParagraphAttributes::default(), false);
            reader.builder.text(trimmed, &code());
            reader.close();
            continue;
        }
        let content = trimmed.trim_start();
        if content.is_empty() {
            reader.close();
            reader.gap = true;
            continue;
        }
        if let Some(opened) = fence(content) {
            reader.close();
            reader.gap = !reader.builder.is_empty();
            reader.start(ParagraphAttributes::default(), false);
            reader.open = false;
            open_fence = Some(opened);
            continue;
        }
        if content
            .chars()
            .all(|character| matches!(character, '-' | '*' | '_' | ' '))
            && content
                .chars()
                .filter(|character| *character != ' ')
                .count()
                >= 3
        {
            reader.close();
            reader.gap = true;
            continue;
        }
        let hashes = content
            .chars()
            .take_while(|character| *character == '#')
            .count();
        if (1..=6).contains(&hashes) && content[hashes..].starts_with(' ') {
            reader.start(ParagraphAttributes::default(), false);
            let title = content[hashes..].trim();
            let closed = title.trim_end_matches('#');
            let title = match closed.is_empty() || closed.ends_with(' ') {
                true => closed.trim_end(),
                false => title,
            };
            inline(&mut reader.builder, title, &heading(hashes), dialect);
            reader.close();
            reader.gap = true;
            continue;
        }
        if let Some((level, marker, rest)) = list_item(trimmed) {
            let attributes = ParagraphAttributes {
                list: Some(marker),
                level: level as u8,
                ..ParagraphAttributes::default()
            };
            reader.start(attributes, true);
            inline(
                &mut reader.builder,
                rest,
                &TextAttributes::default(),
                dialect,
            );
            reader.close();
            continue;
        }
        if let Some(quoted) = content.strip_prefix('>') {
            let attributes = ParagraphAttributes {
                level: 1,
                ..ParagraphAttributes::default()
            };
            if reader.open && *reader.builder.paragraph() == attributes {
                reader.builder.text(" ", &TextAttributes::default());
            } else {
                reader.gap |= reader.open;
                reader.start(attributes, false);
            }
            let italic = TextAttributes {
                italic: true,
                ..TextAttributes::default()
            };
            inline(&mut reader.builder, quoted.trim(), &italic, dialect);
            continue;
        }
        match reader.open && !reader.listing {
            true => reader.builder.text(" ", &TextAttributes::default()),
            false => reader.start(ParagraphAttributes::default(), false),
        }
        inline(
            &mut reader.builder,
            content,
            &TextAttributes::default(),
            dialect,
        );
    }
    reader.close();
    reader.builder.finish()
}

fn is_list_number(text: &str) -> bool {
    (1..=MAX_NUMBER_DIGITS).contains(&text.len()) && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_foreign_autolink(written: &str) -> bool {
    let word = written
        .rsplit(|character: char| !character.is_ascii_alphabetic())
        .next()
        .unwrap_or_default();
    FOREIGN_AUTOLINKS
        .iter()
        .any(|scheme| word.eq_ignore_ascii_case(scheme))
}

fn escape(text: &str, dialect: Dialect, line_start: usize, out: &mut String) {
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        let next = characters.peek();
        let special = match character {
            '\\' | '*' | '_' | '`' | '[' | ']' | '~' => true,
            _ if dialect == Dialect::Authored => false,
            '<' | '#' => true,
            '&' => next.is_none_or(|next| *next == '#' || next.is_ascii_alphanumeric()),
            '!' => next.is_none(),
            '-' | '+' | '=' | '>' => out.len() == line_start,
            ':' => is_foreign_autolink(&out[line_start..]),
            '.' | ')' => is_list_number(&out[line_start..]),
            _ => false,
        };
        if special {
            out.push('\\');
        }
        out.push(character);
    }
}

fn is_code_block(paragraph: &Paragraph<'_>) -> bool {
    paragraph.attributes.list.is_none()
        && !paragraph.is_empty()
        && paragraph
            .spans
            .iter()
            .all(|(_, attributes)| is_code(attributes))
}

fn span_target(span: &TextAttributes, dialect: Dialect) -> Option<String> {
    let url = span.link.as_deref()?;
    match dialect {
        Dialect::Authored if destination_end(&format!("{url})"), 0) == Some(url.len()) => {
            Some(url.to_owned())
        }
        Dialect::Authored => Some(percent_encoded(url)),
        Dialect::Sanitised => allowed(url).map(|url| percent_encoded(&url).replace('&', "&amp;")),
    }
}

fn is_marked(
    marker: &str,
    span: &TextAttributes,
    paragraph: ParagraphAttributes,
    heading: bool,
) -> bool {
    match marker {
        "**" => span.bold && !heading,
        "*" => span.italic && (paragraph.level == 0 || paragraph.list.is_some()),
        _ => span.strikethrough,
    }
}

fn line(paragraph: &Paragraph<'_>, number: usize, dialect: Dialect) -> String {
    let mut out = String::new();
    let attributes = paragraph.attributes;
    let level = heading_level(&paragraph.first());
    match (attributes.list, level) {
        (Some(ListMarker::Bullet), _) => {
            out.push_str(&"  ".repeat(usize::from(attributes.level)));
            out.push_str("- ");
        }
        (Some(ListMarker::Number), _) => {
            out.push_str(&"  ".repeat(usize::from(attributes.level)));
            out.push_str(&format!("{number}. "));
        }
        (None, Some(level)) => {
            out.push_str(&"#".repeat(level));
            out.push(' ');
        }
        (None, None) if attributes.level > 0 => out.push_str("> "),
        (None, None) => {}
    }
    let line_start = out.len();
    let mut open: Vec<&str> = Vec::new();
    let mut gap = String::new();
    for (index, (text, span)) in paragraph.spans.iter().enumerate() {
        let text: Cow<'_, str> = match dialect {
            Dialect::Authored => Cow::Borrowed(text),
            Dialect::Sanitised => text.chars().map(printable).collect(),
        };
        let shown = match is_code(span) {
            true => &text,
            false => text.trim(),
        };
        if shown.is_empty() {
            gap.push_str(&text);
            continue;
        }
        let (before, after) = match is_code(span) {
            true => ("", ""),
            false => (
                &text[..text.len() - text.trim_start().len()],
                &text[text.trim_end().len()..],
            ),
        };
        let kept = open
            .iter()
            .take_while(|marker| is_marked(marker, span, attributes, level.is_some()))
            .count();
        out.extend(open.drain(kept..).rev());
        if out.len() > line_start || dialect == Dialect::Authored {
            out.push_str(&gap);
            out.push_str(before);
        }
        gap.clear();
        gap.push_str(after);
        let mut opened: Vec<&str> = MARKERS
            .into_iter()
            .filter(|marker| {
                !open.contains(marker) && is_marked(marker, span, attributes, level.is_some())
            })
            .collect();
        opened.sort_by_key(|marker| {
            Reverse(
                paragraph.spans[index..]
                    .iter()
                    .take_while(|(_, span)| is_marked(marker, span, attributes, level.is_some()))
                    .count(),
            )
        });
        out.extend(opened.iter().copied());
        open.append(&mut opened);
        match (span_target(span, dialect), is_code(span)) {
            (_, true) => {
                out.push('`');
                out.push_str(&shown.replace('`', "'"));
                out.push('`');
            }
            (Some(url), false) => {
                out.push('[');
                escape(shown, dialect, line_start, &mut out);
                out.push_str("](");
                out.push_str(&url);
                out.push(')');
            }
            (None, false) => escape(shown, dialect, line_start, &mut out),
        }
    }
    out.extend(open.into_iter().rev());
    if dialect == Dialect::Authored {
        out.push_str(&gap);
    }
    out
}

pub fn write(rich: &RichText) -> String {
    compose(rich, Dialect::Authored)
}

/// Writes Markdown that any CommonMark reader shows without images, raw HTML, a metadata block or
/// links to anything but `https`, `http` and `mailto`.
pub fn write_sanitised(rich: &RichText) -> String {
    compose(rich, Dialect::Sanitised)
        .chars()
        .map(|character| match character {
            '\n' => character,
            _ => printable(character),
        })
        .collect()
}

fn longest_backticks(text: &str) -> usize {
    text.split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or_default()
}

fn write_block(code: &str, out: &mut String) {
    let fence = "`".repeat(MIN_FENCE.max(longest_backticks(code) + 1));
    out.push_str(&format!("{fence}\n{code}{fence}\n"));
}

fn code_line(paragraph: &Paragraph<'_>) -> impl Iterator<Item = char> {
    paragraph
        .spans
        .iter()
        .flat_map(|(text, _)| text.chars())
        .chain(['\n'])
}

fn code_blocks(paragraphs: &[Paragraph<'_>]) -> Vec<bool> {
    let mut next_code = false;
    let mut blocks: Vec<bool> = paragraphs
        .iter()
        .rev()
        .map(|paragraph| {
            if !paragraph.is_empty() {
                next_code = is_code_block(paragraph);
            }
            next_code
        })
        .collect();
    blocks.reverse();
    blocks
}

fn compose(rich: &RichText, dialect: Dialect) -> String {
    let paragraphs = paragraphs(rich);
    let blocks = code_blocks(&paragraphs);
    let mut out = String::new();
    let mut previous_list = false;
    let mut gap = false;
    let mut block: Option<String> = None;
    let mut numbers = [0usize; MAX_LEVEL + 1];
    for (paragraph, is_code) in paragraphs.iter().zip(blocks) {
        if let Some(code) = block.as_mut().filter(|_| is_code) {
            code.extend(code_line(paragraph));
            continue;
        }
        if let Some(code) = block.take() {
            write_block(&code, &mut out);
        }
        if paragraph.is_empty() {
            gap = true;
            continue;
        }
        let listed = paragraph.attributes.list.is_some();
        if !out.is_empty() && (gap || !(previous_list && listed)) {
            out.push('\n');
        }
        let gap_before = std::mem::take(&mut gap);
        if is_code {
            block = Some(code_line(paragraph).collect());
            previous_list = false;
            continue;
        }
        let level = usize::from(paragraph.attributes.level).min(MAX_LEVEL);
        if paragraph.attributes.list != Some(ListMarker::Number) || gap_before {
            numbers[level] = 0;
        }
        let number = match paragraph.attributes.list {
            Some(_) => {
                numbers[level] += 1;
                numbers[level + 1..].fill(0);
                numbers[level]
            }
            None => {
                numbers = [0; MAX_LEVEL + 1];
                0
            }
        };
        out.push_str(&line(paragraph, number, dialect));
        out.push('\n');
        previous_list = listed;
    }
    if let Some(code) = block {
        write_block(&code, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitised_markdown_carries_no_image_html_metadata_or_other_link() {
        let kept = "# C\\#\n\nSome **bold**, *italic*, `<b>` and a [link](https://example.org/a?b=1&amp;c=2).\n\n- one\n- two\n\n```\n<b> & ![x](y)\n```\n\n[mail](mailto:a@example.org)\n";
        assert_eq!(sanitise(kept), kept);

        for (source, expected) in [
            ("---\ntitle: x\n---\ntext", "text\n"),
            ("\u{feff}---\r\ntitle: x\r\n...\r\ntext", "text\n"),
            ("\\---\na: b\n\\---", "\\--- a: b ---\n"),
            ("a ![p](https://e.org/p.png) b", "a p b\n"),
            ("\\![p](https://e.org)", "\\![p](https://e.org)\n"),
            ("<img src=x onerror=y>", "\\<img src=x onerror=y>\n"),
            ("a <!-- b", "a \\<!-- b\n"),
            ("&#60;b> R&D", "\\&\\#60;b> R\\&D\n"),
            (
                "<https://e.org> <file:///x>",
                "[https://e.org](https://e.org) \\<file:///x>\n",
            ),
            ("[x](JavaScript:y) [x](/etc) [x](file:///x)", "x x x\n"),
            ("[x](java&#115;cript:y) [x](<data:y> \"t\")", "x x\n"),
            (
                "[x](<HTTPS://e.org/a b> \"t\")",
                "[x](HTTPS://e.org/a%20b)\n",
            ),
            (
                "[x](https://e.org/(a)\\<b>)",
                "[x](https://e.org/%28a%29%3Cb%3E)\n",
            ),
            (
                "[x][r]\n\n[r]: javascript:y",
                "\\[x\\]\\[r\\]\n\n\\[r\\]: javascript:y\n",
            ),
            ("````\n```\n<img>\n````", "````\n```\n<img>\n````\n"),
            ("[a [b]](https://e.org)", "[a \\[b\\]](https://e.org)\n"),
            ("a\u{0}\rb", "a b\n"),
            ("a\u{2028}b", "a b\n"),
            (
                "see FTP://e.org or https://e.org",
                "see FTP\\://e.org or https://e.org\n",
            ),
            ("\u{0}\u{feff}---\na: b\n---\ntext", "text\n"),
            (
                "[![p](https://e.org/p.png)](https://e.org)",
                "[p](https://e.org)\n",
            ),
            ("# **T** 2**8", "# T 2\\*\\*8\n"),
            ("*`x`, y*\n> a_b", "*`x`, y*\n\n> a\\_b\n"),
        ] {
            assert_eq!(sanitise(source), expected, "{source}");
            assert_eq!(sanitise(expected), expected, "{source}");
        }

        let mut builder = Builder::default();
        builder.text("\u{feff}---", &TextAttributes::default());
        builder.end_paragraph(ParagraphAttributes::default());
        builder.text("!", &TextAttributes::default());
        builder.text("x", &link("https://e.org/) <img src=x>"));
        builder.text("y", &link("javascript:z"));
        builder.end_paragraph(ParagraphAttributes::default());
        builder.text("```", &code());
        builder.end_paragraph(ParagraphAttributes::default());
        builder.text("<img>", &code());
        assert_eq!(
            write_sanitised(&builder.finish()),
            "\\---\n\n\\![x](https://e.org/%29%20%3Cimg%20src=x%3E)y\n\n````\n```\n<img>\n````\n"
        );

        for hostile in [
            "[](<",
            "<",
            "[](a \"",
            "[\\](",
            "![",
            "\\",
            "[]((",
            "```\n\n\n\n",
            "[[]",
            "`x`\n\n``",
        ] {
            assert!(sanitise(&hostile.repeat((1 << 17) / hostile.len())).len() < 1 << 20);
        }
    }
}
