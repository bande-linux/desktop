pub mod system;

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, OnceLock};

use bande_l10n_source::catalog::Catalog;
use bande_l10n_source::pattern::render;

pub use bande_l10n_macros::{text, tr, trn};

pub const CATALOGS: &str = "/usr/share/bande/l10n";
const LOCALE_VARIABLES: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];
const REGION_VARIABLES: [&str; 4] = ["LC_ALL", "LC_TIME", "LC_MEASUREMENT", "LANG"];

static PROGRAM: LazyLock<Catalogs> = LazyLock::new(|| {
    Catalogs::open(
        &Path::new(CATALOGS).join(program_name()),
        &requested_languages(|name| std::env::var(name).ok()),
    )
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Text {
    context: Option<&'static str>,
    english: &'static str,
}

impl Text {
    #[doc(hidden)]
    pub const fn new(context: Option<&'static str>, english: &'static str) -> Self {
        Self { context, english }
    }

    pub fn tr(self) -> String {
        translate(self.context, self.english, &[])
    }

    pub fn tr_args(self, args: &[(&str, &dyn Display)]) -> String {
        translate(self.context, self.english, args)
    }
}

pub trait Count {
    fn count(&self) -> u64;
}

macro_rules! counts {
    ($($unsigned:ty),* ; $($signed:ty),*) => {
        $(impl Count for $unsigned {
            fn count(&self) -> u64 {
                u64::try_from(*self).unwrap_or(u64::MAX)
            }
        })*
        $(impl Count for $signed {
            fn count(&self) -> u64 {
                u64::try_from(self.unsigned_abs()).unwrap_or(u64::MAX)
            }
        })*
    };
}

counts!(u8, u16, u32, u64, usize; i8, i16, i32, i64, isize);

#[doc(hidden)]
pub fn translate(context: Option<&str>, english: &str, args: &[(&str, &dyn Display)]) -> String {
    PROGRAM.translate(context, english, None, args)
}

#[doc(hidden)]
pub fn translate_plural(
    context: Option<&str>,
    singular: &str,
    plural: &str,
    count: u64,
    args: &[(&str, &dyn Display)],
) -> String {
    PROGRAM.translate(context, singular, Some((plural, count)), args)
}

struct Catalogs {
    folder: PathBuf,
    installed: Vec<String>,
    chain: Vec<(String, OnceLock<Option<Catalog>>)>,
    others: Vec<String>,
    development: OnceLock<Option<Catalog>>,
}

impl Catalogs {
    fn open(folder: &Path, requested: &[String]) -> Self {
        let installed = installed_in(folder);
        let is_installed = |tag: &str| installed.iter().any(|language| language == tag);
        let chosen = requested.iter().find_map(|wanted| {
            std::iter::successors(Some(wanted.as_str()), |tag| parent(tag))
                .find(|tag| is_installed(tag))
        });
        let chain = std::iter::successors(chosen, |tag| parent(tag))
            .filter(|tag| is_installed(tag))
            .map(|tag| (tag.to_owned(), OnceLock::new()))
            .collect::<Vec<_>>();
        let others = installed
            .iter()
            .filter(|language| chain.iter().all(|(tag, _)| tag != *language))
            .cloned()
            .collect();
        Self {
            folder: folder.to_owned(),
            installed,
            chain,
            others,
            development: OnceLock::new(),
        }
    }

    fn development(&self) -> Option<&Catalog> {
        self.development
            .get_or_init(|| {
                let chosen = self.chain.iter().any(|(_, catalog)| {
                    catalog
                        .get()
                        .and_then(Option::as_ref)
                        .is_some_and(Catalog::is_development)
                });
                if chosen {
                    return None;
                }
                self.others
                    .iter()
                    .filter_map(|tag| self.load(tag))
                    .find(Catalog::is_development)
            })
            .as_ref()
    }

    fn catalogs(&self) -> impl Iterator<Item = &Catalog> {
        self.chain
            .iter()
            .filter_map(|(tag, catalog)| catalog.get_or_init(|| self.load(tag)).as_ref())
            .chain(std::iter::once_with(|| self.development()).flatten())
    }

    fn load(&self, tag: &str) -> Option<Catalog> {
        let path = self.folder.join(format!("{tag}.mo"));
        let data = std::fs::read(&path).ok()?;
        Catalog::parse(data)
            .inspect_err(|error| tracing::warn!("cannot read {}: {error}", path.display()))
            .ok()
    }

    fn translate(
        &self,
        context: Option<&str>,
        english: &str,
        plural: Option<(&str, u64)>,
        args: &[(&str, &dyn Display)],
    ) -> String {
        self.catalogs()
            .find_map(|catalog| {
                let forms = catalog.find(context, english)?;
                let form = plural.map_or(0, |(_, count)| catalog.plural().index(count));
                render(forms.get(form)?, args)
            })
            .unwrap_or_else(|| {
                let source = match plural {
                    Some((plural, count)) if count != 1 => plural,
                    _ => english,
                };
                render(source, args).unwrap_or_else(|| source.to_owned())
            })
    }
}

fn installed_in(folder: &Path) -> Vec<String> {
    let mut languages = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_suffix(".mo").map(str::to_owned)
        })
        .collect::<Vec<_>>();
    languages.sort();
    languages
}

pub fn installed_languages() -> &'static [String] {
    &PROGRAM.installed
}

fn parent(tag: &str) -> Option<&str> {
    tag.rsplit_once('-').map(|(parent, _)| parent)
}

fn program_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_name()?.to_str().map(str::to_owned))
        .map(|name| name.trim_end_matches(" (deleted)").to_owned())
        .unwrap_or_default()
}

pub fn preferred_languages() -> Vec<String> {
    requested_languages(|name| std::env::var(name).ok())
        .into_iter()
        .filter_map(|tag| tag.split('-').next().map(str::to_owned))
        .collect()
}

pub fn region_language() -> Option<String> {
    let locale = REGION_VARIABLES
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))?;
    if matches!(locale.as_str(), "C" | "POSIX") || locale.starts_with("C.") {
        return None;
    }
    language_of_locale(&locale).and_then(|tag| tag.split('-').next().map(str::to_owned))
}

fn requested_languages(var: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let Some(locale) = LOCALE_VARIABLES
        .into_iter()
        .find_map(|name| var(name).filter(|value| !value.is_empty()))
    else {
        return Vec::new();
    };
    if matches!(locale.as_str(), "C" | "POSIX") {
        return Vec::new();
    }
    let preferred = var("LANGUAGE").unwrap_or_default();
    let mut tags = Vec::<String>::new();
    for tag in preferred
        .split(':')
        .chain([locale.as_str()])
        .filter_map(language_of_locale)
    {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

fn language_of_locale(locale: &str) -> Option<String> {
    let name = locale.split(['.', '@']).next()?;
    let valid = !name.is_empty()
        && name.split(['_', '-']).all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        });
    valid.then(|| name.replace('_', "-"))
}

#[cfg(test)]
mod tests {
    use bande_l10n_source::DEVELOPMENT_HEADER;
    use bande_l10n_source::catalog::{Entry, write};

    use super::*;

    fn entry(english: &str, plural: Option<&str>, forms: &[&str]) -> Entry {
        Entry {
            context: None,
            english: english.to_owned(),
            plural: plural.map(str::to_owned),
            forms: forms.iter().map(|form| form.to_string()).collect(),
        }
    }

    #[test]
    fn a_catalog_serves_only_its_english_and_the_code_covers_the_rest() {
        let folder = std::env::temp_dir().join(format!("bande-l10n-test-{}", std::process::id()));
        let catalogs = |requested: &[&str]| {
            let requested = requested
                .iter()
                .map(|tag| tag.to_string())
                .collect::<Vec<_>>();
            Catalogs::open(&folder, &requested)
        };
        let made = std::fs::create_dir_all(&folder).and_then(|()| {
            std::fs::write(
                folder.join("qaa.mo"),
                write(
                    "Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n==2 ? 1 : 2);\n",
                    &[
                        entry("Greeting", None, &["Qa"]),
                        entry(
                            "{count} file",
                            Some("{count} files"),
                            &["{count} qa", "{count} qb", "{count} qc"],
                        ),
                        entry("Hello, {name}", None, &["Qa, {name}"]),
                        entry("Untranslated", None, &[""]),
                    ],
                ),
            )?;
            std::fs::write(
                folder.join("qaa-QB.mo"),
                write("", &[entry("Greeting", None, &["Qb"])]),
            )?;
            std::fs::write(
                folder.join("en.mo"),
                write(
                    &format!("{DEVELOPMENT_HEADER}: yes\n"),
                    &[entry(
                        "{count} file",
                        Some("{count} files"),
                        &["one {count}", "{count} files"],
                    )],
                ),
            )
        });
        assert!(made.is_ok());
        let qaa = catalogs(&["qaa-QA"]);
        assert_eq!(qaa.translate(None, "Greeting", None, &[]), "Qa");
        assert_eq!(
            qaa.translate(
                None,
                "{count} file",
                Some(("{count} files", 2)),
                &[("count", &2)]
            ),
            "2 qb"
        );
        assert_eq!(
            qaa.translate(None, "Hello, {name}", None, &[("name", &"Ada")]),
            "Qa, Ada"
        );
        assert_eq!(
            qaa.translate(None, "Greeting, changed", None, &[]),
            "Greeting, changed"
        );
        assert_eq!(
            qaa.translate(None, "Untranslated", None, &[]),
            "Untranslated"
        );
        assert_eq!(
            qaa.translate(Some("door"), "Greeting", None, &[]),
            "Greeting"
        );
        let regional = catalogs(&["qaa-QB"]);
        assert_eq!(regional.translate(None, "Greeting", None, &[]), "Qb");
        assert_eq!(
            regional.translate(None, "Hello, {name}", None, &[("name", &"Ada")]),
            "Qa, Ada"
        );
        let missing = catalogs(&["de"]);
        assert_eq!(
            missing.translate(
                None,
                "{count} file",
                Some(("{count} files", 1)),
                &[("count", &1)]
            ),
            "one 1"
        );
        assert_eq!(missing.translate(None, "Greeting", None, &[]), "Greeting");
        assert!(std::fs::remove_dir_all(&folder).is_ok());
    }
}
