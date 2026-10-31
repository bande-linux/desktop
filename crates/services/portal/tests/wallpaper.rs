#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bande_config::{ConfigService, ConfigStore};
use bande_xdg::file_uri;
use common::Portal;
use xdg_desktop_portal_bande::OBJECT_PATH;
use xdg_desktop_portal_bande::request::Cancellation;
use xdg_desktop_portal_bande::wallpaper::{Wallpaper, WallpaperUi};
use zbus::zvariant::{ObjectPath, Value};

const INTERFACE: &str = "org.freedesktop.impl.portal.Wallpaper";

struct FakeUi {
    answer: bool,
    asked: Mutex<Vec<(String, PathBuf)>>,
}

impl WallpaperUi for FakeUi {
    fn confirm(&self, app_id: &str, picture: &Path, _cancellation: &Cancellation) -> bool {
        self.asked
            .lock()
            .unwrap()
            .push((app_id.to_owned(), picture.to_path_buf()));
        self.answer
    }
}

struct Fixture {
    portal: Portal,
    ui: Arc<FakeUi>,
    picture: PathBuf,
    folder: PathBuf,
    _config: ConfigService,
    _directory: tempfile::TempDir,
}

fn start(answer: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config.kdl");
    std::fs::write(&config, "").unwrap();
    let picture = directory.path().join("Downloads/sunset.jpg");
    std::fs::create_dir_all(picture.parent().unwrap()).unwrap();
    std::fs::write(&picture, b"a picture").unwrap();
    let folder = directory.path().join("share/backgrounds");
    let ui = Arc::new(FakeUi {
        answer,
        asked: Mutex::new(Vec::new()),
    });
    let mut service = None;
    let portal = Portal::start(|connection| {
        service = Some(
            ConfigService::start(
                connection,
                &config,
                Arc::new(ConfigStore::default()),
                |_| {},
            )
            .unwrap(),
        );
        connection
            .object_server()
            .at(
                OBJECT_PATH,
                Wallpaper {
                    ui: ui.clone(),
                    folder: folder.clone(),
                    session: connection.clone(),
                },
            )
            .unwrap();
    });
    Fixture {
        portal,
        ui,
        picture,
        folder,
        _config: service.unwrap(),
        _directory: directory,
    }
}

impl Fixture {
    fn set(&self, uri: &str, preview: bool) -> u32 {
        let handle = ObjectPath::from_static_str_unchecked("/org/freedesktop/portal/desktop/w/1");
        let options = HashMap::from([("show-preview", Value::from(preview))]);
        self.portal
            .call(
                INTERFACE,
                "SetWallpaperURI",
                &(handle, "org.gnome.Loupe", "", uri, options),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap()
    }

    fn wallpaper(&self) -> (Option<PathBuf>, Option<PathBuf>) {
        let choice = bande_wallpaper::read(&self.portal.client);
        (choice.light, choice.dark)
    }

    fn kept(&self) -> Vec<PathBuf> {
        std::fs::read_dir(&self.folder)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default()
    }
}

#[test]
fn a_confirmed_picture_is_kept_and_becomes_the_desktop_picture() {
    let fixture = start(true);

    assert_eq!(fixture.set(&file_uri(&fixture.picture), true), 0);

    assert_eq!(
        *fixture.ui.asked.lock().unwrap(),
        [("org.gnome.Loupe".to_owned(), fixture.picture.clone())]
    );
    let kept = fixture.kept();
    assert_eq!(kept.len(), 1);
    assert!(
        kept[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-sunset.jpg")
    );
    assert_eq!(std::fs::read(&kept[0]).unwrap(), b"a picture");
    assert_eq!(
        fixture.wallpaper(),
        (Some(kept[0].clone()), Some(kept[0].clone()))
    );
}

#[test]
fn a_declined_preview_changes_nothing() {
    let fixture = start(false);

    assert_eq!(fixture.set(&file_uri(&fixture.picture), true), 1);

    assert_eq!(fixture.ui.asked.lock().unwrap().len(), 1);
    assert!(fixture.kept().is_empty());
    assert_eq!(fixture.wallpaper(), (None, None));
}

#[test]
fn without_a_preview_the_picture_is_set_directly() {
    let fixture = start(false);

    assert_eq!(fixture.set(&file_uri(&fixture.picture), false), 0);

    assert!(fixture.ui.asked.lock().unwrap().is_empty());
    assert_eq!(fixture.kept().len(), 1);
}

#[test]
fn only_local_pictures_can_be_set() {
    let fixture = start(true);

    assert_eq!(fixture.set("https://example.org/sunset.jpg", true), 2);
    assert_eq!(fixture.set("file:///nowhere/sunset.jpg", true), 2);

    assert!(fixture.ui.asked.lock().unwrap().is_empty());
    assert_eq!(fixture.wallpaper(), (None, None));
}
