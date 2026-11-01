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
