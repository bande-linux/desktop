use std::collections::HashSet;
use std::sync::Mutex;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

const FALLBACK: &str = "en-US";
const LOCALE_VARIABLES: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

pub struct Localizer {
    bundles: Vec<FluentBundle<FluentResource>>,
    reported: Mutex<HashSet<String>>,
}

impl Localizer {
    pub fn new(translations: &[(&str, &str)]) -> Self {
        Self::negotiated(
            translations,
            &requested_languages(|name| std::env::var(name).ok()),
        )
    }

    fn negotiated(translations: &[(&str, &str)], requested: &[LanguageIdentifier]) -> Self {
        let available = translations
            .iter()
            .filter_map(|(tag, source)| match tag.parse::<LanguageIdentifier>() {
                Ok(language) => Some((language, *source)),
                Err(error) => {
                    tracing::warn!("cannot use the translation {tag}: {error}");
                    None
                }
            })
            .collect::<Vec<_>>();
        let languages = available
            .iter()
            .map(|(language, _)| language.clone())
            .collect::<Vec<_>>();
        let bundles = bundle_order(requested, &languages)
            .into_iter()
            .map(|index| bundle(available[index].0.clone(), available[index].1))
            .collect();
        Self {
            bundles,
            reported: Mutex::new(HashSet::new()),
        }
    }

    pub fn tr(&self, id: &str) -> String {
        self.format(id, None)
    }

    pub fn tr_args<'a>(
        &self,
        id: &str,
        args: impl IntoIterator<Item = (&'a str, FluentValue<'a>)>,
    ) -> String {
        self.format(id, Some(&args.into_iter().collect()))
    }

    fn format(&self, id: &str, args: Option<&FluentArgs>) -> String {
        let found = self.bundles.iter().find_map(|bundle| {
            let pattern = bundle.get_message(id)?.value()?;
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, args, &mut errors);
            if !errors.is_empty() {
                tracing::warn!("cannot format the message {id}: {errors:?}");
            }
            Some(text.into_owned())
        });
        found.unwrap_or_else(|| {
            if let Ok(mut reported) = self.reported.lock()
                && reported.insert(id.to_owned())
            {
                tracing::warn!("no translation for the message {id}");
            }
            id.to_owned()
        })
    }

    fn has(&self, id: &str) -> bool {
        self.bundles.iter().any(|bundle| bundle.has_message(id))
    }
}

pub fn missing_ids<'a>(english: &str, sources: &[&'a str]) -> Vec<&'a str> {
    let localizer = Localizer::negotiated(&[(FALLBACK, english)], &[]);
    sources
        .iter()
        .flat_map(|source| used_ids(source))
        .filter(|id| !localizer.has(id))
        .collect()
}

fn used_ids(source: &str) -> impl Iterator<Item = &str> {
    ["tr(\"", "tr_args(\""].into_iter().flat_map(move |call| {
        source
            .match_indices(call)
            .filter(|(start, _)| {
                !source[..*start]
                    .ends_with(|letter: char| letter.is_alphanumeric() || letter == '_')
            })
            .filter_map(move |(start, _)| source[start + call.len()..].split('"').next())
    })
}

fn bundle(language: LanguageIdentifier, source: &str) -> FluentBundle<FluentResource> {
    let resource =
        FluentResource::try_new(source.to_owned()).unwrap_or_else(|(resource, errors)| {
            tracing::warn!("errors in the {language} translation: {errors:?}");
            resource
        });
    let mut bundle = FluentBundle::new_concurrent(vec![language]);
    bundle.set_use_isolating(false);
    if let Err(errors) = bundle.add_resource(resource) {
        tracing::warn!("errors in the translation: {errors:?}");
    }
    bundle
}

fn requested_languages(var: impl Fn(&str) -> Option<String>) -> Vec<LanguageIdentifier> {
    let Some(locale) = LOCALE_VARIABLES
        .into_iter()
        .find_map(|name| var(name).filter(|value| !value.is_empty()))
    else {
        return Vec::new();
    };
    if matches!(locale.as_str(), "C" | "POSIX") || locale.starts_with("C.") {
        return Vec::new();
    }
    let preferred = var("LANGUAGE").unwrap_or_default();
    preferred
        .split(':')
        .chain([locale.as_str()])
        .filter_map(language_of_locale)
        .collect()
}

fn language_of_locale(locale: &str) -> Option<LanguageIdentifier> {
    let name = locale.split(['.', '@']).next()?;
    name.replace('_', "-").parse().ok()
}

fn bundle_order(requested: &[LanguageIdentifier], available: &[LanguageIdentifier]) -> Vec<usize> {
    let chosen = requested.iter().find_map(|wanted| {
        available
            .iter()
            .position(|language| language == wanted)
            .or_else(|| {
                available
                    .iter()
                    .position(|language| language.language == wanted.language)
            })
    });
    let fallback = available.iter().position(|language| *language == FALLBACK);
    let mut order = Vec::from_iter(chosen);
    order.extend(fallback.filter(|index| chosen != Some(*index)));
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENGLISH: &str = "greeting = Hello, { $name }!\n\
        items = { $count ->\n    [one] 1 item\n   *[other] { $count } items\n}\n\
        only-english = Only in English\n";
    const GERMAN: &str = "greeting = Hallo, { $name }!\n";

    fn environment(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        }
    }

    fn tags(languages: &[LanguageIdentifier]) -> Vec<String> {
        languages.iter().map(ToString::to_string).collect()
    }

    fn localizer(env: &[(&str, &str)]) -> Localizer {
        Localizer::negotiated(
            &[("en-US", ENGLISH), ("de-DE", GERMAN)],
            &requested_languages(environment(env)),
        )
    }

    #[test]
    fn requested_languages_follow_language_then_lc_all_lc_messages_and_lang() {
        for (env, expected) in [
            (
                &[("LANGUAGE", "fr_CA:de"), ("LANG", "sv_SE.UTF-8")][..],
                &["fr-CA", "de", "sv-SE"][..],
            ),
            (
                &[
                    ("LC_ALL", "de_AT.UTF-8@euro"),
                    ("LC_MESSAGES", "fr_FR.UTF-8"),
                    ("LANG", "sv_SE.UTF-8"),
                ],
                &["de-AT"],
            ),
            (
                &[("LC_MESSAGES", "fr_FR.UTF-8"), ("LANG", "sv_SE.UTF-8")],
                &["fr-FR"],
            ),
            (&[("LANGUAGE", "de"), ("LANG", "C.UTF-8")], &[]),
        ] {
            assert_eq!(
                tags(&requested_languages(environment(env))),
                expected,
                "{env:?}"
            );
        }
    }

    #[test]
    fn a_regional_variant_picks_the_same_language() {
        let german = localizer(&[("LANG", "de_AT.UTF-8")]);
        assert_eq!(
            german.tr_args("greeting", [("name", "Ada".into())]),
            "Hallo, Ada!"
        );
    }

    #[test]
    fn an_unavailable_language_falls_back_to_english() {
        let swedish = localizer(&[("LANGUAGE", "sv:fi"), ("LANG", "sv_SE.UTF-8")]);
        assert_eq!(
            swedish.tr_args("greeting", [("name", "Ada".into())]),
            "Hello, Ada!"
        );
    }

    #[test]
    fn a_message_missing_in_the_language_comes_from_english() {
        let german = localizer(&[("LANG", "de_DE.UTF-8")]);
        assert_eq!(german.tr("only-english"), "Only in English");
    }

    #[test]
    fn counts_choose_the_plural_form() {
        let english = localizer(&[]);
        assert_eq!(english.tr_args("items", [("count", 1.into())]), "1 item");
        assert_eq!(english.tr_args("items", [("count", 3.into())]), "3 items");
    }

    #[test]
    fn a_missing_message_shows_its_id() {
        assert_eq!(localizer(&[]).tr("no-such-message"), "no-such-message");
    }

    #[test]
    fn ids_used_in_code_are_checked_against_the_english_file() {
        let source = "let a = L10N.tr(\"greeting\");\n\
            let b = L10N.tr_args(\"unknown\", [(\"count\", 1.into())]);\n\
            let c = attr(\"not-an-id\");";
        assert_eq!(missing_ids(ENGLISH, &[source]), ["unknown"]);
    }
}
