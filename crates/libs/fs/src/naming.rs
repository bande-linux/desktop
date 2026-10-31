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
