use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Keyword,
    Literal,
    Number,
    Comment,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Code {
    pub text: String,
    pub tones: Vec<(Range<usize>, Tone)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading(u8, String),
    Paragraph(String),
    Bullet(String),
    Quote(String),
    Code(String),
    Plain(String),
    Rule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comments {
    Slashes,
    Hash,
    Dashes,
    None,
}

const KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "def",
    "default",
    "defer",
    "del",
    "do",
    "elif",
    "else",
    "enum",
    "esac",
    "except",
    "export",
    "extends",
    "extern",
    "false",
    "fi",
    "finally",
    "fn",
    "for",
    "from",
    "func",
    "function",
    "go",
    "if",
    "impl",
    "import",
    "in",
    "interface",
    "is",
    "lambda",
    "let",
    "local",
    "loop",
    "match",
    "mod",
    "module",
    "mut",
    "namespace",
    "new",
    "nil",
    "None",
    "not",
    "null",
    "or",
    "package",
    "private",
    "protected",
    "pub",
    "public",
    "raise",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "switch",
    "then",
    "this",
    "throw",
    "trait",
    "True",
    "False",
    "true",
    "try",
    "type",
    "typedef",
    "union",
    "unsafe",
    "use",
    "using",
    "var",
    "void",
    "where",
    "while",
    "with",
    "yield",
];

fn comments(extension: &str) -> Option<Comments> {
    Some(match extension {
        "rs" | "c" | "h" | "cc" | "cpp" | "hpp" | "cxx" | "js" | "mjs" | "ts" | "tsx" | "jsx"
        | "java" | "go" | "swift" | "kt" | "cs" | "css" | "scss" | "dart" | "zig" | "m"
        | "json" | "jsonc" | "wgsl" | "glsl" | "proto" | "kdl" | "php" | "scala" => {
            Comments::Slashes
        }
        "py" | "sh" | "bash" | "zsh" | "fish" | "rb" | "pl" | "toml" | "yaml" | "yml" | "conf"
        | "ini" | "cfg" | "cmake" | "mk" | "r" | "nix" | "desktop" | "service" | "dockerfile"
        | "makefile" => Comments::Hash,
        "sql" | "lua" | "hs" | "elm" | "ada" => Comments::Dashes,
        "xml" | "svg" | "csv" | "tsv" | "diff" | "patch" | "log" => Comments::None,
        _ => return None,
    })
}

pub fn is_code(extension: &str) -> bool {
    comments(&extension.to_ascii_lowercase()).is_some()
}

pub fn highlight(text: String, extension: &str) -> Code {
    let Some(comments) = comments(&extension.to_ascii_lowercase()) else {
        return Code {
            text,
            tones: Vec::new(),
        };
    };
    let bytes = text.as_bytes();
    let mut tones = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let rest = &bytes[at..];
        let line_end = |from: usize| {
            bytes[from..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| from + offset)
        };
        let comment = match comments {
            Comments::Slashes if rest.starts_with(b"//") => Some(line_end(at)),
            Comments::Slashes if rest.starts_with(b"/*") => Some(
                text[at + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |offset| at + 2 + offset + 2),
            ),
            Comments::Hash if rest[0] == b'#' => Some(line_end(at)),
            Comments::Dashes if rest.starts_with(b"--") => Some(line_end(at)),
            _ => None,
        };
        if let Some(end) = comment {
            tones.push((at..end, Tone::Comment));
            at = end;
            continue;
        }
        let byte = rest[0];
        if byte == b'"' || byte == b'\'' && comments != Comments::Hash || byte == b'`' {
            let mut end = at + 1;
            while end < bytes.len() && bytes[end] != byte && bytes[end] != b'\n' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            let end = (end + 1).min(bytes.len());
            if byte != b'\'' || end - at > 3 || bytes.get(end - 1) == Some(&b'\'') {
                tones.push((at..end, Tone::Literal));
            }
            at = end;
            continue;
        }
        if byte.is_ascii_alphanumeric() || byte == b'_' {
            let end = rest
                .iter()
                .position(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'.'))
                .map_or(bytes.len(), |offset| at + offset);
            let word = &text[at..end];
            if byte.is_ascii_digit() {
                tones.push((at..end, Tone::Number));
            } else if KEYWORDS.contains(&word) {
                tones.push((at..end, Tone::Keyword));
            }
            at = end;
            continue;
        }
        at += text[at..].chars().next().map_or(1, char::len_utf8);
    }
    Code { text, tones }
}

pub fn markdown(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph = String::new();
    let mut fence: Option<String> = None;
    let flush = |paragraph: &mut String, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(inline(&std::mem::take(paragraph))));
        }
    };
    for line in text.lines() {
        if let Some(code) = &mut fence {
            if line.trim_start().starts_with("```") {
                blocks.push(Block::Code(std::mem::take(code)));
                fence = None;
            } else {
                if !code.is_empty() {
                    code.push('\n');
                }
                code.push_str(line);
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(&mut paragraph, &mut blocks);
            fence = Some(String::new());
        } else if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
        } else if let Some(level) = heading_level(trimmed) {
            flush(&mut paragraph, &mut blocks);
            let title = trimmed[level as usize..]
                .trim()
                .trim_end_matches('#')
                .trim();
            blocks.push(Block::Heading(level, inline(title)));
        } else if trimmed.len() >= 3
            && trimmed
                .chars()
                .all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Rule);
        } else if let Some(item) = bullet(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Bullet(inline(item)));
        } else if let Some(quoted) = trimmed.strip_prefix('>') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Quote(inline(quoted.trim())));
        } else if trimmed.starts_with('|') {
            flush(&mut paragraph, &mut blocks);
            match blocks.last_mut() {
                Some(Block::Code(table)) if table.starts_with('|') => {
                    table.push('\n');
                    table.push_str(trimmed);
                }
                _ => blocks.push(Block::Code(trimmed.to_owned())),
            }
        } else if line.starts_with("    ") || line.starts_with('\t') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Code(line.trim_start().to_owned()));
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(trimmed);
        }
    }
    if let Some(code) = fence {
        blocks.push(Block::Code(code));
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

fn heading_level(line: &str) -> Option<u8> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    ((1..=6).contains(&level) && line[level..].starts_with(' ')).then_some(level as u8)
}

fn bullet(line: &str) -> Option<&str> {
    if let Some(rest) = ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| line.strip_prefix(marker))
    {
        return Some(rest);
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0)
        .then(|| line[digits..].strip_prefix(". "))
        .flatten()
}

fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    let mut previous = ' ';
    while let Some(character) = characters.next() {
        let inside_word = previous.is_alphanumeric()
            && characters.peek().is_some_and(|next| next.is_alphanumeric());
        previous = character;
        match character {
            '_' if inside_word => out.push('_'),
            '*' | '_' | '`' => {}
            '!' if characters.peek() == Some(&'[') => {}
            '[' => {
                let label: String = characters.by_ref().take_while(|c| *c != ']').collect();
                out.push_str(&label);
                if characters.peek() == Some(&'(') {
                    characters.by_ref().take_while(|c| *c != ')').for_each(drop);
                }
            }
            '\\' => out.extend(characters.next()),
            _ => out.push(character),
        }
    }
    out
}

pub fn rtf(text: &str) -> Vec<Block> {
    let mut out = String::new();
    let mut skipped = Vec::new();
    let mut skipping = false;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '{' => skipped.push(skipping),
            '}' => skipping = skipped.pop().unwrap_or(false),
            '\\' => {
                let mut word = String::new();
                while let Some(c) = characters.peek().copied().filter(char::is_ascii_alphabetic) {
                    word.push(c);
                    characters.next();
                }
                if word.is_empty() {
                    match characters.next() {
                        Some('*') => skipping = true,
                        Some('\'') => {
                            let hex: String = characters.by_ref().take(2).collect();
                            if let Ok(byte) = u8::from_str_radix(&hex, 16)
                                && !skipping
                            {
                                out.push(char::from(byte));
                            }
                        }
                        Some(c) if !skipping => out.push(c),
                        _ => {}
                    }
                    continue;
                }
                let mut number = String::new();
                while let Some(c) = characters
                    .peek()
                    .copied()
                    .filter(|c| c.is_ascii_digit() || *c == '-')
                {
                    number.push(c);
                    characters.next();
                }
                if characters.peek() == Some(&' ') {
                    characters.next();
                }
                match word.as_str() {
                    "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict" | "header"
                    | "footer" | "expandedcolortbl" => skipping = true,
                    "par" | "line" if !skipping => out.push('\n'),
                    "tab" if !skipping => out.push('\t'),
                    "u" if !skipping => {
                        if let Some(c) = number
                            .parse::<i32>()
                            .ok()
                            .and_then(|code| char::from_u32(code.rem_euclid(65536) as u32))
                        {
                            out.push(c);
                        }
                        if characters.peek() == Some(&'?') {
                            characters.next();
                        }
                    }
                    _ => {}
                }
            }
            '\r' | '\n' => {}
            c if !skipping => out.push(c),
            _ => {}
        }
    }
    out.split('\n')
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(|line| Block::Paragraph(line.to_owned()))
        .collect()
}

pub fn html(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut current = String::new();
    let mut make: fn(String) -> Block = Block::Paragraph;
    let mut hidden = 0usize;
    let mut preformatted = false;
    let mut rest = text;
    let finish = |current: &mut String, make: fn(String) -> Block, blocks: &mut Vec<Block>| {
        let text = current.trim().to_owned();
        current.clear();
        if !text.is_empty() {
            blocks.push(make(text));
        }
    };
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            if hidden == 0 {
                push_text(&mut current, rest, preformatted);
            }
            break;
        };
        if hidden == 0 {
            push_text(&mut current, &rest[..open], preformatted);
        }
        rest = &rest[open..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map_or("", |end| &rest[end + 3..]);
            continue;
        }
        let close = rest.find('>').unwrap_or(rest.len() - 1);
        let tag = rest[1..close].trim().to_ascii_lowercase();
        rest = &rest[close + 1..];
        let ending = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        match name.as_str() {
            "script" | "style" | "head" | "title" | "svg" | "noscript" => match ending {
                true => hidden = hidden.saturating_sub(1),
                false => hidden += 1,
            },
            "br" => current.push('\n'),
            "hr" => {
                finish(&mut current, make, &mut blocks);
                blocks.push(Block::Rule);
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "main" | "nav"
            | "table" | "tr" | "ul" | "ol" | "body" | "form" => {
                finish(&mut current, make, &mut blocks);
                make = Block::Paragraph;
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                finish(&mut current, make, &mut blocks);
                make = match (ending, name.as_bytes()[1] - b'0') {
                    (true, _) => Block::Paragraph,
                    (false, 1) => |text| Block::Heading(1, text),
                    (false, 2) => |text| Block::Heading(2, text),
                    (false, 3) => |text| Block::Heading(3, text),
                    (false, _) => |text| Block::Heading(4, text),
                };
            }
            "li" => {
                finish(&mut current, make, &mut blocks);
                make = if ending {
                    Block::Paragraph
                } else {
                    Block::Bullet
                };
            }
            "blockquote" => {
                finish(&mut current, make, &mut blocks);
                make = if ending {
                    Block::Paragraph
                } else {
                    Block::Quote
                };
            }
            "pre" => {
                finish(&mut current, make, &mut blocks);
                preformatted = !ending;
                make = if ending {
                    Block::Paragraph
                } else {
                    Block::Code
                };
            }
            "td" | "th" if ending => current.push('\t'),
            _ => {}
        }
    }
    finish(&mut current, make, &mut blocks);
    blocks
}

fn push_text(current: &mut String, text: &str, preformatted: bool) {
    let decoded = entities(text);
    if preformatted {
        current.push_str(&decoded);
        return;
    }
    let spaced = |current: &String| !current.is_empty() && !current.ends_with(['\n', ' ', '\t']);
    if decoded.starts_with(char::is_whitespace) && spaced(current) {
        current.push(' ');
    }
    for (index, word) in decoded.split_whitespace().enumerate() {
        if index > 0 {
            current.push(' ');
        }
        current.push_str(word);
    }
    if decoded.ends_with(char::is_whitespace) && spaced(current) {
        current.push(' ');
    }
}

fn entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.find(';').filter(|end| *end <= 10);
        let decoded = end.and_then(|end| {
            let name = &rest[1..end];
            match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                _ => name
                    .strip_prefix("#x")
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                    .and_then(char::from_u32),
            }
        });
        match (decoded, end) {
            (Some(character), Some(end)) => {
                out.push(character);
                rest = &rest[end + 1..];
            }
            _ => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}
