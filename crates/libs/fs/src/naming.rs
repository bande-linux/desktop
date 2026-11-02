use std::path::{Path, PathBuf};

use crate::L10N;

pub(crate) fn split_extension(name: &str, is_dir: bool) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => name.split_at(dot),
        _ => (name, ""),
    }
}

fn free(folder: &Path, names: impl IntoIterator<Item = String>) -> PathBuf {
    names
        .into_iter()
        .map(|name| folder.join(name))
        .find(|path| path.symlink_metadata().is_err())
        .unwrap_or_else(|| folder.to_path_buf())
}

fn numbered(stem: &str) -> (&str, u64) {
    stem.rsplit_once(' ')
        .and_then(|(base, number)| {
            let parsed = number.parse::<u64>().ok()?;
            (!base.is_empty() && !number.starts_with('0') && parsed >= 2).then_some((base, parsed))
        })
        .unwrap_or((stem, 1))
}

pub fn duplicate_name(folder: &Path, name: &str, is_dir: bool) -> PathBuf {
    let (stem, extension) = split_extension(name, is_dir);
    let first = L10N.tr_args("copy-name", [("name", stem.into())]);
    let later = (2u64..).map(|number| {
        L10N.tr_args(
            "copy-name-numbered",
            [("name", stem.into()), ("number", number.to_string().into())],
        )
    });
    free(
        folder,
        std::iter::once(first)
            .chain(later)
            .map(|candidate| format!("{candidate}{extension}")),
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
            (Some(from), Some(to)) => L10N.tr_args(
                "extension-change-title",
                [("from", from.into()), ("to", to.into())],
            ),
            (None, to) => L10N.tr_args(
                "extension-add-title",
                [("extension", to.unwrap_or_default().into())],
            ),
            (Some(from), None) => {
                L10N.tr_args("extension-remove-title", [("extension", from.into())])
            }
        }
    }

    pub fn detail() -> String {
        L10N.tr("extension-detail")
    }

    pub fn keep(&self) -> String {
        match (&self.from, &self.to) {
            (Some(from), Some(_)) => {
                L10N.tr_args("extension-keep", [("extension", from.clone().into())])
            }
            (None, _) => L10N.tr("extension-dont-add"),
            (Some(_), None) => L10N.tr("extension-dont-remove"),
        }
    }

    pub fn change(&self) -> String {
        match (&self.from, &self.to) {
            (Some(_), Some(to)) => {
                L10N.tr_args("extension-use", [("extension", to.clone().into())])
            }
            (None, _) => L10N.tr("extension-add"),
            (Some(_), None) => L10N.tr("extension-remove"),
        }
    }
}

pub fn alias_name(folder: &Path, name: &str) -> PathBuf {
    let first = L10N.tr_args("alias-name", [("name", name.into())]);
    let later = (2u64..).map(|number| {
        L10N.tr_args(
            "alias-name-numbered",
            [("name", name.into()), ("number", number.to_string().into())],
        )
    });
    free(folder, std::iter::once(first).chain(later))
}

pub fn keep_both_name(folder: &Path, name: &str, is_dir: bool) -> PathBuf {
    let (stem, extension) = split_extension(name, is_dir);
    let (base, number) = numbered(stem);
    free(
        folder,
        (number + 1..).map(|number| format!("{base} {number}{extension}")),
    )
}

pub fn untitled_folder(folder: &Path) -> PathBuf {
    let name = L10N.tr("untitled-folder");
    free(
        folder,
        std::iter::once(name.clone()).chain((2u64..).map(|number| format!("{name} {number}"))),
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
