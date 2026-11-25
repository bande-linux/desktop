#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use bande_config::{ConfigService, ConfigStore};
use bande_test_intents::{Change, Owned, stands};
use tempfile::TempDir;
use zbus::blocking::Connection;

const PROGRAM: &str = env!("CARGO_BIN_EXE_bande-settings-intents");

struct Config {
    folder: TempDir,
    _service: ConfigService,
    _connection: Connection,
}

impl Config {
    fn serve(connection: Connection) -> Self {
        let folder = tempfile::tempdir().expect("a folder");
        let path = folder.path().join("config.kdl");
        std::fs::write(&path, "appearance \"dark\"\n").expect("config");
        let service =
            ConfigService::start(&connection, &path, Arc::new(ConfigStore::default()), |_| {})
                .expect("config service");
        Self {
            folder,
            _service: service,
            _connection: connection,
        }
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.folder.path().join("config.kdl")).unwrap_or_default()
    }
}

fn bande(owned: &Owned, line: &str) -> Result<String, String> {
    let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
    bande_cli::run(&owned.manifests, || Ok(owned.connect()), &words)
}

#[test]
fn a_setting_is_changed_on_the_command_line_and_undone_after_the_owner_restarts() {
    let mut owned = Owned::start(PROGRAM);
    let config = Config::serve(owned.connect());
    let help = bande(&owned, "settings set --help").unwrap();
    assert!(
        help.starts_with("bande settings set <setting> <value>"),
        "{help}"
    );
    assert!(
        bande(&owned, "settings")
            .unwrap()
            .contains("bande settings open-pane")
    );
    assert!(
        bande(&owned, "settings find theme")
            .unwrap()
            .contains("appearance")
    );
    bande(&owned, "settings set appearance light").unwrap();
    assert!(
        config.text().contains("appearance \"light\""),
        "{}",
        config.text()
    );
    owned.restart();
    bande(&owned, "undo").unwrap();
    assert!(!config.text().contains("\"light\""), "{}", config.text());
    assert!(bande(&owned, "undo").is_err(), "an undo is not undone");

    bande(&owned, "settings set appearance light").unwrap();
    bande(&owned, "settings set appearance dark").unwrap();
    bande(&owned, "undo").unwrap();
    assert!(
        config.text().contains("\"light\""),
        "changes are undone in order"
    );

    let refused = bande(&owned, "settings set appearance purple").unwrap_err();
    assert!(refused.contains("light"), "{refused}");
    assert!(bande(&owned, "settings set lock-message hello").is_err());
    let asked = bande(&owned, "settings set automatic-updates install --json").unwrap_err();
    assert!(asked.contains("unavailable"), "{asked}");
}

#[test]
fn the_owner_stands_what_every_owner_must() {
    stands(
        Path::new(PROGRAM),
        |bus| {
            let config = Config::serve(bus.connect().expect("a connection"));
            move || config.text()
        },
        &Change {
            find: "settings.find",
            intent: "settings.set",
            words: &["appearance", "light"],
        },
    );
}
