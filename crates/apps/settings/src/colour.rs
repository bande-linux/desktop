use std::fs;
use std::path::{Path, PathBuf};

use bande_app::choose_file;
use bande_colour::{IccError, PROFILE_FOLDER, Profile};
use zbus::blocking::Connection;

use crate::L10N;

const SYSTEM_FOLDER: &str = "color/icc";

fn folders() -> Vec<PathBuf> {
    bande_xdg::data_home()
        .map(|home| home.join(PROFILE_FOLDER))
        .into_iter()
        .chain(
            bande_xdg::data_dirs()
                .into_iter()
                .map(|dir| dir.join(SYSTEM_FOLDER)),
        )
        .collect()
}

fn usable(path: &Path) -> bool {
    fs::read(path).is_ok_and(|bytes| Profile::parse_icc(&bytes).is_ok())
}

fn label(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().replace(['_', '-'], " "))
        .unwrap_or_default()
}

pub(crate) fn installed() -> Vec<(String, String)> {
    let mut profiles: Vec<(String, String)> = folders()
        .iter()
        .filter_map(|folder| fs::read_dir(folder).ok())
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| {
                extension.eq_ignore_ascii_case("icc") || extension.eq_ignore_ascii_case("icm")
            })
        })
        .filter(|path| usable(path))
        .map(|path| (label(&path), path.to_string_lossy().into_owned()))
        .collect();
    profiles.sort();
    profiles.dedup_by(|a, b| a.0 == b.0);
    profiles
}

fn reason(error: &IccError) -> String {
    L10N.tr(match error {
        IccError::TooShort => "icc-too-short",
        IccError::NotADisplayProfile => "icc-not-display",
        IccError::NotRgb => "icc-not-rgb",
        IccError::NotXyz => "icc-not-xyz",
        IccError::LookupTables => "icc-lookup-tables",
        IccError::MissingTag(_) => "icc-missing-tag",
        IccError::UnsupportedCurve => "icc-unsupported-curve",
    })
}

pub(crate) enum Added {
    Profile(String),
    Refused(String),
    Cancelled,
}

pub(crate) fn add(session: &Connection) -> Added {
    let filter = (
        L10N.tr("colour-profiles"),
        vec![(0u32, "*.icc".to_owned()), (0u32, "*.icm".to_owned())],
    );
    let chosen = match choose_file(session, &L10N.tr("choose-colour-profile"), Some(filter)) {
        Ok(Some(chosen)) => chosen,
        Ok(None) => return Added::Cancelled,
        Err(error) => return Added::Refused(error.to_string()),
    };
    let name = chosen
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let refused = |why: String| {
        Added::Refused(L10N.tr_args(
            "colour-profile-refused",
            [("file", name.clone().into()), ("reason", why.into())],
        ))
    };
    let bytes = match fs::read(&chosen) {
        Ok(bytes) => bytes,
        Err(error) => return refused(error.to_string()),
    };
    if let Err(error) = Profile::parse_icc(&bytes) {
        return refused(reason(&error));
    }
    let Some(folder) = bande_xdg::data_home().map(|home| home.join(PROFILE_FOLDER)) else {
        return refused(L10N.tr("icc-no-home"));
    };
    let stored = folder.join(&name);
    match fs::create_dir_all(&folder).and_then(|()| fs::write(&stored, &bytes)) {
        Ok(()) => Added::Profile(stored.to_string_lossy().into_owned()),
        Err(error) => refused(error.to_string()),
    }
}
