use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use bande_l10n::tr;

const NAME_MARK: &str = "\u{F0000}";

pub(crate) fn split_extension(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => name.split_at(dot),
        _ => (name, ""),
    }
}

fn split_os_extension(name: &OsStr, is_dir: bool) -> (&OsStr, &OsStr) {
    let bytes = name.as_bytes();
    match bytes.iter().rposition(|byte| *byte == b'.') {
        Some(dot) if dot > 0 && !is_dir => (
            OsStr::from_bytes(&bytes[..dot]),
            OsStr::from_bytes(&bytes[dot..]),
        ),
        _ => (name, OsStr::new("")),
    }
}

fn free(folder: &Path, names: impl IntoIterator<Item = OsString>) -> PathBuf {
    names
        .into_iter()
        .map(|name| folder.join(name))
        .find(|path| path.symlink_metadata().is_err())
        .unwrap_or_else(|| folder.to_path_buf())
}

fn numbered(stem: &OsStr) -> (&OsStr, u64) {
    let bytes = stem.as_bytes();
    bytes
        .iter()
        .rposition(|byte| *byte == b' ')
        .and_then(|space| {
            let number = std::str::from_utf8(&bytes[space + 1..]).ok()?;
            let parsed = number.parse::<u64>().ok()?;
            (space > 0 && !number.starts_with('0') && parsed >= 2)
                .then_some((OsStr::from_bytes(&bytes[..space]), parsed))
        })
        .unwrap_or((stem, 1))
}

fn joined(parts: &[&OsStr]) -> OsString {
    parts.iter().fold(OsString::new(), |mut name, part| {
        name.push(part);
        name
    })
}

fn with_name(template: &str, name: &OsStr) -> OsString {
    let template = template.as_bytes();
    let mark = NAME_MARK.as_bytes();
    let mut filled = Vec::with_capacity(template.len() + name.len());
    let mut rest = template;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix(mark) {
            filled.extend_from_slice(name.as_bytes());
            rest = after;
        } else {
            filled.push(rest[0]);
            rest = &rest[1..];
        }
    }
    OsString::from_vec(filled)
}

pub fn duplicate_name(folder: &Path, name: impl AsRef<OsStr>, is_dir: bool) -> PathBuf {
    let (stem, extension) = split_os_extension(name.as_ref(), is_dir);
    let first = tr!("{name} copy", name = NAME_MARK);
    let later = (2u64..).map(|number| {
        tr!(
            "{name} copy {number}",
            name = NAME_MARK,
            number = number.to_string()
        )
    });
    free(
        folder,
        std::iter::once(first)
            .chain(later)
            .map(|candidate| joined(&[&with_name(&candidate, stem), extension])),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionChange {
    pub from: Option<String>,
    pub to: Option<String>,
    pub kept: String,
}

impl ExtensionChange {
    pub fn of(path: &Path, name: &str) -> Option<Self> {
        let extension = |path: &Path| {
            path.extension()
                .map(|extension| format!(".{}", extension.to_string_lossy()))
        };
        let (from, to) = (extension(path), extension(Path::new(name)));
        if from == to {
            return None;
        }
        let stem = Path::new(name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let kept = format!("{stem}{}", from.as_deref().unwrap_or_default());
        Some(Self { from, to, kept })
    }

    pub fn title(&self) -> String {
        let (from, to) = (self.from.clone(), self.to.clone());
        match (from, to) {
            (Some(from), Some(to)) => tr!(
                "Are you sure you want to change the extension from “{from}” to “{to}”?",
                from = from,
                to = to
            ),
            (None, to) => tr!(
                "Are you sure you want to add the extension “{extension}” to the end of the name?",
                extension = to.unwrap_or_default()
            ),
            (Some(from), None) => {
                tr!(
                    "Are you sure you want to remove the extension “{extension}”?",
                    extension = from
                )
            }
        }
    }

    pub fn detail() -> String {
        tr!("If you make this change, your document may open in a different app.")
    }

    pub fn keep(&self) -> String {
        match (&self.from, &self.to) {
            (Some(from), Some(_)) => {
                tr!("Keep {extension}", extension = from.clone())
            }
            (None, _) => tr!("Don’t Add"),
            (Some(_), None) => tr!("Keep"),
        }
    }

    pub fn change(&self) -> String {
        match (&self.from, &self.to) {
            (Some(_), Some(to)) => {
                tr!("Use {extension}", extension = to.clone())
            }
            (None, _) => tr!("Add"),
            (Some(_), None) => tr!("Remove"),
        }
    }
}

pub fn alias_name(folder: &Path, name: &OsStr) -> PathBuf {
    let first = tr!("{name} alias", name = NAME_MARK);
    let later = (2u64..).map(|number| {
        tr!(
            "{name} alias {number}",
            name = NAME_MARK,
            number = number.to_string()
        )
    });
    free(
        folder,
        std::iter::once(first)
            .chain(later)
            .map(|candidate| with_name(&candidate, name)),
    )
}

pub fn keep_both_name(folder: &Path, name: impl AsRef<OsStr>, is_dir: bool) -> PathBuf {
    let (stem, extension) = split_os_extension(name.as_ref(), is_dir);
    let (base, number) = numbered(stem);
    free(
        folder,
        (number + 1..).map(|number| joined(&[base, OsStr::new(&format!(" {number}")), extension])),
    )
}

pub fn untitled_folder(folder: &Path) -> PathBuf {
    let name = tr!("untitled folder");
    free(
        folder,
        std::iter::once(name.clone())
            .chain((2u64..).map(|number| format!("{name} {number}")))
            .map(OsString::from),
    )
}

pub fn typed_location(typed: &str, home: &Path, folder: &Path) -> PathBuf {
    let typed = typed.trim();
    let expanded = match typed.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(typed),
    };
    folder.join(expanded)
}
