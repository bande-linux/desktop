use std::borrow::Cow;

use unic_langid::LanguageIdentifier;

const LANGUAGE: &str = "en-XA";
const PLAIN: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ACCENTED: &str = "ȦƁƇḒḖƑƓĦĪĴĶĿḾȠǾƤɊŘŞŦŬṼẆẊẎẐȧƀƈḓḗƒɠħīĵķŀḿƞǿƥɋřşŧŭṽẇẋẏẑ";
const DOUBLED: &str = "AEIOUSYaeiousy";

pub fn is_requested(language: &LanguageIdentifier) -> bool {
    language.to_string() == LANGUAGE
}

pub fn accented(text: &str) -> Cow<'_, str> {
    let mut pseudo = String::with_capacity(text.len() * 2);
    for character in text.chars() {
        let shown = PLAIN
            .find(character)
            .and_then(|index| ACCENTED.chars().nth(index))
            .unwrap_or(character);
        pseudo.push(shown);
        if DOUBLED.contains(character) {
            pseudo.push(shown);
        }
    }
    Cow::Owned(pseudo)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn english_files(folder: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for path in entries.flatten().map(|entry| entry.path()) {
            if path.is_dir() && !path.ends_with("target") {
                english_files(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "ftl")
                && path
                    .parent()
                    .is_some_and(|parent| parent.ends_with("en-US"))
            {
                found.push(path);
            }
        }
    }

    #[test]
    fn letters_become_accented_and_vowels_and_s_doubled() {
        assert_eq!(accented("Open files…"), "ǾǾƥḗḗƞ ƒīīŀḗḗşş…");
    }

    #[test]
    fn the_english_messages_grow_about_a_third() {
        let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        english_files(&crates, &mut files);
        let english = files
            .iter()
            .filter_map(|file| std::fs::read_to_string(file).ok())
            .flat_map(|source| {
                source
                    .lines()
                    .filter_map(|line| line.split_once(" = ").map(|(_, value)| value.to_owned()))
                    .collect::<Vec<_>>()
            })
            .collect::<String>();
        let grown = accented(&english).chars().count() as f64 / english.chars().count() as f64;
        assert!(!files.is_empty());
        assert!((1.3..1.45).contains(&grown), "grows by {grown}");
    }
}
