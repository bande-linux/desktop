use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use web_content::Pixels;

use crate::engine;
use crate::report::Tally;

pub const GROUPS: [&str; 4] = [
    "css/CSS2",
    "css/css-cascade",
    "css/css-syntax",
    "css/selectors",
];

pub const WIDTH: u16 = 800;
pub const HEIGHT: u16 = 600;

struct Reftest {
    group: usize,
    path: PathBuf,
    matches: Vec<PathBuf>,
    mismatches: Vec<PathBuf>,
    fuzzy: Vec<(Option<String>, Fuzzy)>,
}

#[derive(Clone, Copy)]
struct Fuzzy {
    max_difference: (u8, u8),
    total_pixels: (usize, usize),
}

const EXACT: Fuzzy = Fuzzy {
    max_difference: (0, 0),
    total_pixels: (0, 0),
};

pub fn run(wpt: &Path) -> Result<Vec<Tally>, String> {
    let mut tests = Vec::new();
    for (group, dir) in GROUPS.iter().enumerate() {
        for path in crate::files(&wpt.join(dir), "")? {
            if let Some(test) = discover(wpt, group, path) {
                tests.push(test);
            }
        }
    }
    let tallies = Mutex::new(GROUPS.map(|dir| Tally::new(format!("wpt reftests {dir}"))));
    let next = AtomicUsize::new(0);
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                while let Some(test) = tests.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let mut blank = false;
                    let passed = crate::guarded(|| test.passes(&mut blank));
                    if let Ok(mut tallies) = tallies.lock()
                        && let Some(tally) = tallies.get_mut(test.group)
                    {
                        tally.record(passed, || test.path.display().to_string());
                        tally.blank += usize::from(passed && blank);
                    }
                }
            });
        }
    });
    let mut tallies = tallies.into_inner().map_err(|error| error.to_string())?;
    for tally in &mut tallies {
        tally.failures.sort();
    }
    Ok(tallies.into())
}

impl Reftest {
    fn passes(&self, blank: &mut bool) -> bool {
        let Some(test) = engine::render(&self.path) else {
            return false;
        };
        *blank = test
            .rgba
            .as_chunks::<4>()
            .0
            .windows(2)
            .all(|pair| pair[0] == pair[1]);
        let compare = |reference: &PathBuf| {
            engine::render(reference).map(|pixels| {
                let fuzzy = self.fuzzy_for(reference);
                let (max, count) = difference(&test, &pixels);
                (max == 0 && count == 0)
                    || ((fuzzy.max_difference.0..=fuzzy.max_difference.1).contains(&max)
                        && (fuzzy.total_pixels.0..=fuzzy.total_pixels.1).contains(&count))
            })
        };
        (self.matches.is_empty() || self.matches.iter().any(|path| compare(path) == Some(true)))
            && self
                .mismatches
                .iter()
                .all(|path| compare(path) == Some(false))
    }

    fn fuzzy_for(&self, reference: &Path) -> Fuzzy {
        let named = self.fuzzy.iter().find(|(name, _)| {
            name.as_deref().is_some_and(|name| {
                self.path
                    .parent()
                    .is_some_and(|dir| dir.join(name) == reference)
            })
        });
        named
            .or_else(|| self.fuzzy.iter().find(|(name, _)| name.is_none()))
            .map_or(EXACT, |(_, fuzzy)| *fuzzy)
    }
}

fn difference(a: &Pixels, b: &Pixels) -> (u8, usize) {
    if a.width != b.width || a.height != b.height || a.rgba.len() != b.rgba.len() {
        return (u8::MAX, usize::MAX);
    }
    let (a, _) = a.rgba.as_chunks::<4>();
    let (b, _) = b.rgba.as_chunks::<4>();
    a.iter().zip(b).fold((0, 0), |(max, count), (a, b)| {
        let pixel = a
            .iter()
            .zip(b)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0);
        (max.max(pixel), count + usize::from(pixel > 0))
    })
}

fn discover(wpt: &Path, group: usize, path: PathBuf) -> Option<Reftest> {
    let extension = path.extension()?.to_str()?;
    if !matches!(extension, "html" | "htm" | "xht" | "xhtml") {
        return None;
    }
    let relative = path.strip_prefix(wpt).ok()?;
    let excluded = relative.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some("support" | "resources" | "reference" | "tools" | "crashtests")
        )
    });
    let stem = path.file_stem()?.to_str()?;
    if excluded || stem.ends_with("-ref") || stem.starts_with("ref-") {
        return None;
    }
    let source = std::fs::read_to_string(&path).ok()?;
    let mut test = Reftest {
        group,
        path,
        matches: Vec::new(),
        mismatches: Vec::new(),
        fuzzy: Vec::new(),
    };
    for (tag, attributes) in tags(&source) {
        let attribute = |name: &str| {
            attributes
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| *value)
        };
        if tag.eq_ignore_ascii_case("link")
            && let (Some(rel), Some(href)) = (attribute("rel"), attribute("href"))
        {
            let target = resolve(wpt, &test.path, href);
            match rel.to_ascii_lowercase().as_str() {
                "match" => test.matches.push(target),
                "mismatch" => test.mismatches.push(target),
                _ => {}
            }
        } else if tag.eq_ignore_ascii_case("meta")
            && attribute("name").is_some_and(|name| name.eq_ignore_ascii_case("fuzzy"))
            && let Some(content) = attribute("content")
            && let Some(fuzzy) = parse_fuzzy(content)
        {
            test.fuzzy.push(fuzzy);
        }
    }
    (!test.matches.is_empty() || !test.mismatches.is_empty()).then_some(test)
}

fn resolve(wpt: &Path, test: &Path, href: &str) -> PathBuf {
    let href = href.split(['?', '#']).next().unwrap_or_default();
    match href.strip_prefix('/') {
        Some(absolute) => wpt.join(absolute),
        None => test
            .parent()
            .map_or_else(|| wpt.join(href), |dir| dir.join(href)),
    }
}

fn parse_fuzzy(content: &str) -> Option<(Option<String>, Fuzzy)> {
    let (reference, values) = match content.rsplit_once(':') {
        Some((reference, values)) => (Some(reference.trim().to_owned()), values),
        None => (None, content),
    };
    let mut parts = values.split(';').map(|part| {
        let value = part.split_once('=').map_or(part, |(_, value)| value).trim();
        match value.split_once('-') {
            Some((low, high)) => Some((low.trim().parse().ok()?, high.trim().parse().ok()?)),
            None => value.parse().ok().map(|exact| (exact, exact)),
        }
    });
    let max_difference = parts.next()??;
    let total_pixels = parts.next()??;
    Some((
        reference,
        Fuzzy {
            max_difference: (
                u8::try_from(max_difference.0).ok()?,
                u8::try_from(max_difference.1).ok()?,
            ),
            total_pixels,
        },
    ))
}

fn tags(source: &str) -> impl Iterator<Item = (&str, Vec<(&str, &str)>)> {
    source.split('<').skip(1).filter_map(|rest| {
        let body = rest.split('>').next()?;
        let name_end = body
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(body.len());
        let (name, mut rest) = body.split_at(name_end);
        let mut attributes = Vec::new();
        loop {
            rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
            if rest.is_empty() {
                break;
            }
            let key_end = rest
                .find(|c: char| c.is_ascii_whitespace() || c == '=')
                .unwrap_or(rest.len());
            let (key, after) = rest.split_at(key_end);
            let after = after.trim_start();
            let Some(value_start) = after.strip_prefix('=') else {
                attributes.push((key, ""));
                rest = after;
                continue;
            };
            let value_start = value_start.trim_start();
            let (value, next) = match value_start.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let inner = &value_start[1..];
                    let end = inner.find(quote).unwrap_or(inner.len());
                    (&inner[..end], inner.get(end + 1..).unwrap_or_default())
                }
                _ => {
                    let end = value_start
                        .find(|c: char| c.is_ascii_whitespace())
                        .unwrap_or(value_start.len());
                    value_start.split_at(end)
                }
            };
            attributes.push((key, value));
            rest = next;
        }
        Some((name, attributes))
    })
}
