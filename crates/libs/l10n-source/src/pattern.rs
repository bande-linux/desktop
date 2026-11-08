use std::fmt::{Display, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Brace(char),
    Placeholder(&'a str),
}

pub fn pieces(pattern: &str) -> Result<Vec<Piece<'_>>, String> {
    let mut pieces = Vec::new();
    let mut rest = pattern;
    while let Some(at) = rest.find(['{', '}']) {
        if at > 0 {
            pieces.push(Piece::Text(&rest[..at]));
        }
        let tail = &rest[at..];
        if let Some(after) = tail.strip_prefix("{{") {
            pieces.push(Piece::Brace('{'));
            rest = after;
        } else if let Some(after) = tail.strip_prefix("}}") {
            pieces.push(Piece::Brace('}'));
            rest = after;
        } else if let Some(after) = tail.strip_prefix('{') {
            let end = after
                .find('}')
                .ok_or_else(|| format!("unclosed {{ in “{pattern}”"))?;
            let name = &after[..end];
            if !is_name(name) {
                return Err(format!(
                    "“{{{name}}}” in “{pattern}” is not a placeholder name"
                ));
            }
            pieces.push(Piece::Placeholder(name));
            rest = &after[end + 1..];
        } else {
            return Err(format!("a lone }} in “{pattern}”; write }}}}"));
        }
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    Ok(pieces)
}

pub fn placeholders(pattern: &str) -> Result<Vec<&str>, String> {
    Ok(pieces(pattern)?
        .into_iter()
        .filter_map(|piece| match piece {
            Piece::Placeholder(name) => Some(name),
            _ => None,
        })
        .collect())
}

pub fn render(pattern: &str, args: &[(&str, &dyn Display)]) -> Option<String> {
    let mut text = String::with_capacity(pattern.len());
    for piece in pieces(pattern).ok()? {
        match piece {
            Piece::Text(part) => text.push_str(part),
            Piece::Brace(brace) => text.push(brace),
            Piece::Placeholder(name) => {
                let (_, value) = args.iter().find(|(arg, _)| *arg == name)?;
                write!(text, "{value}").ok()?;
            }
        }
    }
    Some(text)
}

fn is_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}
