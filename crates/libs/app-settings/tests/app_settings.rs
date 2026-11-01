#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::time::{Duration, Instant};

use bande_app_settings::{SettingsWatch, SettingsWindow, Shared, Stored, flag, pairs};
use bande_toolkit::ui::{Context, Label, Ui};
use bande_toolkit::{Appearance, TextSystem};

#[derive(Debug, Clone, PartialEq, Default)]
struct Example {
    tidy: bool,
}

impl Stored for Example {
    const FILE: &'static str = "example.conf";

    fn parse(text: &str) -> Self {
        let mut example = Self::default();
        for (key, value) in pairs(text) {
            if key == "tidy" {
                example.tidy = flag(value).unwrap_or(example.tidy);
            }
        }
        example
    }

    fn to_text(&self) -> String {
        format!("tidy = {}\n", self.tidy)
    }
}

fn build(appearance: Appearance, _settings: &Shared<Example>) -> Ui {
    Ui::new(
        Box::new(Label::new("Example")),
        Context::new(TextSystem::new(), appearance.palette()),
    )
}

fn woken(watch: &mut SettingsWatch, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if watch.drain() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn saving_leaves_no_staging_file_and_loads_back() {
    let folder = tempfile::tempdir().unwrap();
    let nested = folder.path().join("bande");
    Example { tidy: true }.save(&nested).unwrap();
    assert_eq!(Example::load(&nested), Example { tidy: true });
    let names: Vec<_> = fs::read_dir(&nested)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["example.conf"]);
}

#[test]
fn broken_values_keep_their_defaults() {
    assert_eq!(Example::parse("tidy = yes\nnoise\n"), Example::default());
    assert_eq!(Example::parse("  tidy=true "), Example { tidy: true });
}

#[test]
fn the_watch_wakes_for_its_own_file_only() {
    let folder = tempfile::tempdir().unwrap();
    let mut watch = SettingsWatch::start(folder.path(), Example::FILE).unwrap();
    fs::write(folder.path().join("other.conf"), "tidy = true\n").unwrap();
    assert!(!woken(&mut watch, Duration::from_millis(300)));
    Example { tidy: true }.save(folder.path()).unwrap();
    assert!(woken(&mut watch, Duration::from_secs(5)));
}

#[test]
fn an_open_settings_window_follows_a_change_made_elsewhere_without_writing_back() {
    let folder = tempfile::tempdir().unwrap();
    let mut window = SettingsWindow::new(
        Appearance::default(),
        Some(folder.path().to_path_buf()),
        build,
    );
    assert_eq!(window.settings(), Example::default());
    assert!(!folder.path().join(Example::FILE).exists());
    let file = folder.path().join(Example::FILE);
    fs::write(&file, "tidy = true\nkept = as written\n").unwrap();
    assert!(window.follow(Appearance::default()));
    assert_eq!(window.settings(), Example { tidy: true });
    assert!(!window.follow(Appearance::default()));
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "tidy = true\nkept = as written\n"
    );
}
