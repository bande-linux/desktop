use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use fluent_syntax::ast::Entry;

struct Literal<'a> {
    text: &'a str,
    translated: bool,
}

fn files(folder: &Path, extension: &str, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut paths = entries
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        if path.is_dir() && !path.ends_with("target") {
            files(&path, extension, found);
        } else if path.extension().is_some_and(|found| found == extension) {
            found.push(path);
        }
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn is_identifier(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn is_translation_call(before: &str) -> bool {
    let before = before.trim_end();
    ["tr(", "tr_args("].iter().any(|call| {
        before
            .strip_suffix(call)
            .is_some_and(|rest| !rest.ends_with(is_identifier))
    })
}

fn string_literals(source: &str) -> Vec<Literal<'_>> {
    let bytes = source.as_bytes();
    let mut literals = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                at = source[at..].find('\n').map_or(bytes.len(), |end| at + end);
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                let mut depth = 0;
                while at < bytes.len() {
                    if source[at..].starts_with("/*") {
                        depth += 1;
                        at += 2;
                    } else if source[at..].starts_with("*/") {
                        depth -= 1;
                        at += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        at += 1;
                    }
                }
            }
            b'\'' => {
                at += match (bytes.get(at + 1), bytes.get(at + 2)) {
                    (Some(b'\\'), _) => source[at + 3..].find('\'').map_or(1, |end| end + 4),
                    (_, Some(b'\'')) => 3,
                    _ => source[at + 1..].chars().next().map_or(1, |first| {
                        let after = at + 1 + first.len_utf8();
                        if bytes.get(after) == Some(&b'\'') {
                            after + 1 - at
                        } else {
                            1
                        }
                    }),
                };
            }
            b'r' if !source[..at].ends_with(is_identifier)
                && matches!(bytes.get(at + 1), Some(b'"' | b'#')) =>
            {
                let hashes = source[at + 1..].chars().take_while(|c| *c == '#').count();
                let open = at + 1 + hashes;
                if bytes.get(open) != Some(&b'"') {
                    at += 1;
                    continue;
                }
                let close = format!("\"{}", "#".repeat(hashes));
                let end = source[open + 1..]
                    .find(&close)
                    .map_or(bytes.len(), |end| open + 1 + end);
                literals.push(Literal {
                    text: &source[open + 1..end],
                    translated: false,
                });
                at = end + close.len();
            }
            b'"' => {
                let start = at + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'"' {
                    end += if bytes[end] == b'\\' { 2 } else { 1 };
                }
                literals.push(Literal {
                    text: &source[start..end.min(bytes.len())],
                    translated: is_translation_call(&source[..at]),
                });
                at = end + 1;
            }
            _ => at += 1,
        }
    }
    literals
}

fn matches_pattern(id: &str, pattern: &str) -> bool {
    let mut parts = pattern.split('{').map(|part| match part.split_once('}') {
        Some((_, text)) => text,
        None => part,
    });
    let Some(first) = parts.next() else {
        return false;
    };
    let Some(mut rest) = id.strip_prefix(first) else {
        return false;
    };
    let parts = parts.collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        let found = if index + 1 == parts.len() {
            rest.strip_suffix(part).map(|_| rest.len() - part.len())
        } else {
            rest.find(part)
        };
        let Some(position) = found.filter(|position| *position > 0) else {
            return false;
        };
        rest = &rest[position + part.len()..];
    }
    rest.is_empty()
}

fn references(source: &str) -> BTreeSet<String> {
    source
        .split('{')
        .skip(1)
        .filter_map(|after| {
            let name = after.trim_start().trim_start_matches('-');
            let end = name
                .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
                .unwrap_or(name.len());
            (end > 0 && name[end..].trim_start().starts_with(['}', '.']))
                .then(|| name[..end].to_owned())
        })
        .collect()
}

struct Catalog {
    crate_dir: PathBuf,
    ids: BTreeSet<String>,
    references: BTreeSet<String>,
}

fn catalog(crate_dir: &Path) -> Catalog {
    let mut ftl = Vec::new();
    files(&crate_dir.join("locales/en-US"), "ftl", &mut ftl);
    let mut ids = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    for path in &ftl {
        let source = read(path);
        let resource = fluent_syntax::parser::parse(source.as_str())
            .unwrap_or_else(|(_, errors)| panic!("{}: {errors:?}", path.display()));
        for entry in resource.body {
            let id = match entry {
                Entry::Message(message) => message.id.name.to_owned(),
                Entry::Term(term) => term.id.name.to_owned(),
                _ => continue,
            };
            assert!(ids.insert(id.clone()), "{}: {id} twice", path.display());
        }
        referenced.extend(references(&source));
    }
    Catalog {
        crate_dir: crate_dir.to_path_buf(),
        ids,
        references: referenced,
    }
}

fn catalogs() -> Vec<Catalog> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut ftl = Vec::new();
    files(&crates, "ftl", &mut ftl);
    let crate_dirs = ftl
        .iter()
        .filter_map(|path| {
            let locales = path
                .ancestors()
                .find(|folder| folder.ends_with("locales"))?;
            Some(locales.parent()?.to_path_buf())
        })
        .collect::<BTreeSet<_>>();
    assert!(crate_dirs.len() > 20, "found only {crate_dirs:?}");
    crate_dirs.iter().map(|folder| catalog(folder)).collect()
}

fn sources(crate_dir: &Path) -> Vec<String> {
    let mut rust = Vec::new();
    files(&crate_dir.join("src"), "rs", &mut rust);
    rust.iter().map(|path| read(path)).collect()
}

#[test]
fn every_message_used_in_code_has_an_english_text() {
    let mut missing = Vec::new();
    for catalog in catalogs() {
        for source in sources(&catalog.crate_dir) {
            missing.extend(
                string_literals(&source)
                    .into_iter()
                    .filter(|literal| literal.translated && !catalog.ids.contains(literal.text))
                    .map(|literal| format!("{}: {}", catalog.crate_dir.display(), literal.text)),
            );
        }
    }
    assert!(missing.is_empty(), "no English text for {missing:#?}");
}

#[test]
fn every_english_message_is_used() {
    let mut unused = Vec::new();
    for catalog in catalogs() {
        let sources = sources(&catalog.crate_dir);
        let literals = sources
            .iter()
            .flat_map(|source| string_literals(source))
            .map(|literal| literal.text)
            .collect::<BTreeSet<_>>();
        let patterns = literals
            .iter()
            .filter(|literal| literal.contains('{') && !literal.contains(' '))
            .collect::<Vec<_>>();
        unused.extend(
            catalog
                .ids
                .iter()
                .filter(|id| {
                    !literals.contains(id.as_str())
                        && !catalog.references.contains(id.as_str())
                        && !patterns.iter().any(|pattern| matches_pattern(id, pattern))
                })
                .map(|id| format!("{}: {id}", catalog.crate_dir.display())),
        );
    }
    assert!(unused.is_empty(), "unused English messages {unused:#?}");
}

#[test]
fn literals_are_found_past_comments_chars_and_raw_strings() {
    let source = r##"
        // tr("in-a-comment")
        let quote = '"';
        let escaped = '\'';
        fn life<'a>(x: &'a str) {}
        let raw = r#"tr("raw")"#;
        let shown = L10N.tr("shown");
        let args = L10N.tr_args(
            "with-args",
            [("count", 1.into())],
        );
        let other = attr("not-an-id");
    "##;
    let found = string_literals(source)
        .into_iter()
        .map(|literal| (literal.text, literal.translated))
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        [
            ("tr(\"raw\")", false),
            ("shown", true),
            ("with-args", true),
            ("count", false),
            ("not-an-id", false),
        ]
    );
}

#[test]
fn formatted_ids_match_their_patterns() {
    assert!(matches_pattern("banners-alert", "banners-{}"));
    assert!(matches_pattern("camera-none", "{}-none"));
    assert!(matches_pattern("role-web-browser", "role-{role}"));
    assert!(!matches_pattern("banners-", "banners-{}"));
    assert!(!matches_pattern("camera-some", "{}-none"));
}
