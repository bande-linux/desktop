use std::path::Path;

const LOCALE_SOURCES: &str = "/usr/share/i18n/locales";
const SUPPORTED: &str = "/usr/share/i18n/SUPPORTED";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableLocale {
    pub code: String,
    pub name: String,
}

pub fn locale_name(code: &str, source: &str) -> String {
    let field = |name: &str| {
        source.lines().find_map(|line| {
            let value = line.trim().strip_prefix(name)?;
            value
                .starts_with([' ', '\t'])
                .then(|| value.trim().trim_matches('"').to_owned())
        })
    };
    match (field("language"), field("territory")) {
        (Some(language), Some(territory)) if !language.is_empty() => {
            format!("{language} ({territory})")
        }
        _ => code.to_owned(),
    }
}

pub fn base(code: &str) -> &str {
    code.split(['.', '@']).next().unwrap_or(code)
}

fn available_locale(entry: &str, sources: &Path) -> AvailableLocale {
    let modifier = entry.split_once('@').map(|(_, modifier)| modifier);
    let territory = base(entry);
    let source = modifier.map_or_else(
        || territory.to_owned(),
        |modifier| format!("{territory}@{modifier}"),
    );
    let name = locale_name(
        &source,
        &std::fs::read_to_string(sources.join(&source)).unwrap_or_default(),
    );
    AvailableLocale {
        code: entry.to_owned(),
        name: match modifier {
            Some(modifier) if name != source => format!("{name}, {modifier}"),
            _ => name,
        },
    }
}

pub fn supported_locales(supported: &str, sources: &Path) -> Vec<AvailableLocale> {
    let mut available: Vec<AvailableLocale> = supported
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let entry = fields.next()?;
            (fields.next() == Some("UTF-8") && base(entry).contains('_')).then_some(entry)
        })
        .map(|entry| available_locale(entry, sources))
        .collect();
    available.sort_by_key(|locale| locale.name.to_lowercase());
    available.dedup_by(|a, b| a.code == b.code);
    available
}

pub fn read_supported() -> Vec<AvailableLocale> {
    supported_locales(
        &std::fs::read_to_string(SUPPORTED).unwrap_or_default(),
        Path::new(LOCALE_SOURCES),
    )
}
