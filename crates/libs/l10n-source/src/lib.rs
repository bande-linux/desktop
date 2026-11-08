pub mod catalog;
pub mod pattern;
pub mod plural;
#[cfg(feature = "syntax")]
pub mod syntax;

pub const CONTEXT_SEPARATOR: char = '\u{4}';
pub const DEVELOPMENT_HEADER: &str = "X-Bande-Development";

pub fn key(context: Option<&str>, english: &str) -> String {
    match context {
        Some(context) => format!("{context}{CONTEXT_SEPARATOR}{english}"),
        None => english.to_owned(),
    }
}
