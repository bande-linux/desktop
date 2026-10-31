#[cfg(any(test, feature = "pseudo"))]
mod pseudo;

use std::borrow::Cow;
use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Mutex;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::memoizer::MemoizerKind;
use fluent_bundle::types::{FluentNumber, FluentNumberOptions};
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use icu_decimal::DecimalFormatter;
use icu_decimal::input::Decimal;
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_locale_core::Locale;
use intl_memoizer::Memoizable;
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
        let transform = transform_for(requested);
        let bundles = bundle_order(requested, &languages)
            .into_iter()
            .map(|index| bundle(available[index].0.clone(), available[index].1, transform))
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

    pub fn decimal(&self, value: f64, fraction_digits: usize) -> String {
        let number = FluentNumber::new(
            value,
            FluentNumberOptions {
                minimum_fraction_digits: Some(fraction_digits),
                maximum_fraction_digits: Some(fraction_digits),
                ..FluentNumberOptions::default()
            },
        );
        self.bundles
            .first()
            .and_then(|bundle| bundle.locales.first())
            .and_then(|language| format_decimal(&locale_of(language), &number))
            .unwrap_or_else(|| format!("{value:.fraction_digits$}"))
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

type Transform = fn(&str) -> Cow<'_, str>;

#[cfg(any(test, feature = "pseudo"))]
fn transform_for(requested: &[LanguageIdentifier]) -> Option<Transform> {
    requested
        .first()
        .is_some_and(pseudo::is_requested)
        .then_some(pseudo::accented as Transform)
}

#[cfg(not(any(test, feature = "pseudo")))]
fn transform_for(_requested: &[LanguageIdentifier]) -> Option<Transform> {
    None
}

struct NumberLocale(Locale);

impl Memoizable for NumberLocale {
    type Args = ();
    type Error = Infallible;

    fn construct(language: LanguageIdentifier, _: ()) -> Result<Self, Infallible> {
        Ok(Self(locale_of(&language)))
    }
}

fn locale_of(language: &LanguageIdentifier) -> Locale {
    language
        .to_string()
        .parse()
        .unwrap_or(icu_locale_core::locale!("en-US"))
}

fn format_number<M: MemoizerKind>(value: &FluentValue, memoizer: &M) -> Option<String> {
    let FluentValue::Number(number) = value else {
        return None;
    };
    memoizer
        .with_try_get_threadsafe::<NumberLocale, _, _>((), |locale| {
            format_decimal(&locale.0, number)
        })
        .ok()
        .flatten()
}

fn format_decimal(locale: &Locale, number: &FluentNumber) -> Option<String> {
    let decimal = decimal(number)?;
    let mut options = DecimalFormatterOptions::default();
    options.grouping_strategy = Some(match number.options.use_grouping {
        true => GroupingStrategy::Auto,
        false => GroupingStrategy::Never,
    });
    let formatter = DecimalFormatter::try_new(locale.into(), options).ok()?;
    Some(formatter.format(&decimal).to_string())
}

fn decimal(number: &FluentNumber) -> Option<Decimal> {
    let mut decimal = number.value.to_string().parse::<Decimal>().ok()?;
    let position = |digits: usize| -i16::try_from(digits).unwrap_or(i16::MAX);
    if let Some(most) = number.options.maximum_fraction_digits {
        decimal.round(position(most));
        decimal.trim_end();
    }
    if let Some(least) = number.options.minimum_fraction_digits {
        decimal.pad_end(position(least));
    }
    Some(decimal)
}

fn bundle(
    language: LanguageIdentifier,
    source: &str,
    transform: Option<Transform>,
) -> FluentBundle<FluentResource> {
    let resource =
        FluentResource::try_new(source.to_owned()).unwrap_or_else(|(resource, errors)| {
            tracing::warn!("errors in the {language} translation: {errors:?}");
            resource
        });
    let mut bundle = FluentBundle::new_concurrent(vec![language]);
    bundle.set_use_isolating(false);
    bundle.set_transform(transform);
    bundle.set_formatter(Some(format_number));
    if let Err(errors) = bundle.add_resource(resource) {
        tracing::warn!("errors in the translation: {errors:?}");
    }
    bundle
}

pub fn number(value: f64, most_fraction_digits: usize) -> FluentValue<'static> {
    FluentValue::Number(FluentNumber::new(
        value,
        FluentNumberOptions {
            maximum_fraction_digits: Some(most_fraction_digits),
            ..FluentNumberOptions::default()
        },
    ))
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
    const GERMAN: &str = "greeting = Hallo, { $name }!\nsize = { $value } GB\n";

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
    fn numbers_are_written_the_way_the_language_writes_them() {
        let size = |localizer: &Localizer, value: f64| {
            localizer.tr_args("size", [("value", number(value, 1))])
        };
        let german = localizer(&[("LANG", "de_DE.UTF-8")]);
        assert_eq!(size(&german, 1234.56), "1.234,6 GB");
        assert_eq!(size(&german, 2.0), "2 GB");
        let english = localizer(&[]);
        assert_eq!(
            english.tr_args("items", [("count", 12345.into())]),
            "12,345 items"
        );
        assert_eq!(german.decimal(12.345, 1), "12,3");
        assert_eq!(english.decimal(7.0, 1), "7.0");
    }

    #[test]
    fn the_pseudo_language_accents_the_english_text_but_not_its_arguments() {
        let pseudo = localizer(&[("LANGUAGE", "en-XA"), ("LANG", "en_US.UTF-8")]);
        assert_eq!(
            pseudo.tr_args("greeting", [("name", "Ada".into())]),
            "Ħḗḗŀŀǿǿ, Ada!"
        );
    }
}
