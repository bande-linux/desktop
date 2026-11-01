#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use bande_xdg::{ICON_THEMES, MimeApps, find_icon, handlers, mimeapps_paths};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn default_apps() -> Vec<(String, Option<String>)> {
    fs::read_to_string(repo().join("packaging/bande-desktop/default-apps"))
        .unwrap()
        .lines()
        .filter(|line| line.starts_with(|first: char| first.is_ascii_alphanumeric()))
        .map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            let icon = Some(columns[1]).filter(|icon| *icon != "-");
            (columns[0].to_owned(), icon.map(str::to_owned))
        })
        .collect()
}

fn desktop_files() -> Vec<PathBuf> {
    fs::read_dir(repo().join("crates/apps"))
        .unwrap()
        .filter_map(|app| fs::read_dir(app.unwrap().path().join("data")).ok())
        .flatten()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "desktop")
        })
        .collect()
}

#[test]
fn every_app_we_ship_or_recommend_has_an_icon_in_the_theme() {
    let data = repo().join("data");
    let shipped = desktop_files().into_iter().map(|path| {
        fs::read_to_string(&path)
            .unwrap()
            .lines()
            .find_map(|line| line.strip_prefix("Icon="))
            .unwrap()
            .to_owned()
    });
    let recommended = default_apps().into_iter().filter_map(|(_, icon)| icon);
    for name in shipped.chain(recommended) {
        assert_eq!(
            find_icon(&name, 48, &ICON_THEMES, std::slice::from_ref(&data)),
            Some(data.join(format!("icons/Bande/scalable/apps/{name}.svg"))),
            "{name}"
        );
    }
}

#[test]
fn every_shipped_default_handler_is_an_app_we_ship_or_recommend() {
    let data = tempfile::tempdir().unwrap();
    let applications = data.path().join("applications");
    fs::create_dir_all(&applications).unwrap();
    for path in desktop_files() {
        fs::copy(&path, applications.join(path.file_name().unwrap())).unwrap();
    }
    for (package, icon) in default_apps() {
        let id = icon.unwrap_or(package);
        fs::write(
            applications.join(format!("{id}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName={id}\nExec={id}\n"),
        )
        .unwrap();
    }
    let list = repo().join("packaging/bande-desktop/bande-mimeapps.list");
    fs::copy(&list, applications.join("bande-mimeapps.list")).unwrap();
    let desktops = ["Bande".to_owned()];
    let found = handlers(&[data.path().to_path_buf()], None, &desktops);
    let apps = MimeApps::load(&mimeapps_paths(
        &[],
        &[data.path().to_path_buf()],
        &desktops,
    ));
    let text = fs::read_to_string(&list).unwrap();
    let defaults: Vec<(&str, &str)> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    assert!(defaults.len() > 20);
    for (mime, id) in defaults {
        assert_eq!(
            apps.default_for(&[mime.to_owned()], &found)
                .map(|entry| format!("{};", entry.id)),
            Some(id.to_owned()),
            "{mime}"
        );
    }
}
