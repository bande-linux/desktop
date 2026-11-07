use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};

use bande_search::{BUS_NAME, Donation, Found, OBJECT_PATH};
use bande_test_bus::SessionBus;
use zbus::blocking::Connection;

struct Config {
    excluded: Vec<String>,
}

#[zbus::interface(name = "org.bande.Config")]
impl Config {
    #[zbus(property)]
    fn search_excluded(&self) -> Vec<String> {
        self.excluded.clone()
    }

    #[zbus(property)]
    fn search_shows(&self) -> std::collections::HashMap<String, bool> {
        std::collections::HashMap::new()
    }
}

fn indexer(bus: &SessionBus, home: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_bande-indexer"))
        .env("HOME", home)
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("DBUS_SESSION_BUS_ADDRESS", bus.address())
        .spawn()
        .expect("indexer starts")
}

fn find(connection: &Connection, text: &str) -> Option<Vec<Found>> {
    connection
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.bande.Search"),
            "Find",
            &(text, "", 40u32),
        )
        .ok()?
        .body()
        .deserialize()
        .ok()
}

fn targets(connection: &Connection, text: &str) -> Vec<PathBuf> {
    find(connection, text)
        .unwrap_or_default()
        .into_iter()
        .map(|found| PathBuf::from(found.target))
        .collect()
}

fn appears(connection: &Connection, text: &str, wanted: &Path) -> bool {
    let until = Instant::now() + Duration::from_secs(40);
    while Instant::now() < until {
        if targets(connection, text).iter().any(|path| path == wanted) {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

fn word_document(path: &Path, words: &str) {
    let mut archive = zip::ZipWriter::new(fs::File::create(path).expect("create document"));
    archive
        .start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .expect("start part");
    write!(
        archive,
        "<w:document><w:p><w:r><w:t>{words}</w:t></w:r></w:p></w:document>"
    )
    .expect("write part");
    archive.finish().expect("finish document");
}

#[test]
fn contents_are_found_kept_current_and_survive_a_restart() {
    let home = tempfile::tempdir().expect("temporary home");
    let root = home.path();
    let bus = SessionBus::start().expect("test bus");
    let connection = bus.connect().expect("test bus connection");
    let private = root.join("Private");
    let config = bus.connect().expect("config connection");
    config
        .object_server()
        .at(
            "/org/bande/Config",
            Config {
                excluded: vec![private.to_string_lossy().into_owned()],
            },
        )
        .expect("serve config");
    config
        .request_name("org.bande.Config")
        .expect("config name");

    let documents = root.join("Documents");
    fs::create_dir_all(&documents).expect("documents");
    fs::create_dir_all(&private).expect("private");
    fs::create_dir_all(root.join(".hidden")).expect("hidden");
    fs::create_dir_all(root.join("build")).expect("build");
    fs::write(
        root.join("build/CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55",
    )
    .expect("tag");
    let note = documents.join("note.txt");
    fs::write(&note, "the purple Elephant walked home").expect("note");
    let report = documents.join("report.docx");
    word_document(&report, "quarterly turquoise figures");
    fs::write(root.join(".hidden/x.txt"), "zebrafish").expect("hidden file");
    fs::write(private.join("x.txt"), "zebrafish").expect("private file");
    fs::write(root.join("build/x.txt"), "zebrafish").expect("cache file");
    fs::write(documents.join("blob.txt"), [0u8, 1, 2, 3]).expect("binary");
    fs::write(documents.join("zebra.txt"), "zebrafish").expect("visible");

    let mut running = indexer(&bus, root);
    assert!(appears(&connection, "purple elephant", &note));
    assert!(
        appears(&connection, "turquoise", &report),
        "a Word document's text is found"
    );
    assert!(
        appears(&connection, "elep", &note),
        "the word being typed matches by prefix"
    );
    assert!(appears(&connection, "\"purple elephant\"", &note));
    assert!(appears(
        &connection,
        "zebrafish",
        &documents.join("zebra.txt")
    ));
    assert_eq!(
        targets(&connection, "zebrafish"),
        vec![documents.join("zebra.txt")],
        "hidden, excluded and cache folders stay out"
    );

    let saved = documents.join("later.txt");
    fs::write(&saved, "a saffron afterthought").expect("later");
    assert!(
        appears(&connection, "saffron", &saved),
        "a saved file is found"
    );

    fs::write(&note, "now it says indigo").expect("edit");
    assert!(appears(&connection, "indigo", &note), "an edit is found");
    assert!(
        targets(&connection, "purple").is_empty(),
        "the old text is gone"
    );

    let moved = root.join("Archive");
    fs::rename(&documents, &moved).expect("move folder");
    assert!(
        appears(&connection, "indigo", &moved.join("note.txt")),
        "a moved folder is found at its new place"
    );

    let donated = Donation {
        id: "42".to_owned(),
        kind: "mail".to_owned(),
        title: "Lunch on Friday".to_owned(),
        detail: "Ada".to_owned(),
        target: "mid:42@example".to_owned(),
        body: "the vermilion place".to_owned(),
    };
    connection
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.bande.Search"),
            "Donate",
            &("org.bande.Mail", vec![donated], "42"),
        )
        .expect("donate");
    assert!(appears(
        &connection,
        "vermilion",
        Path::new("mid:42@example")
    ));

    running.kill().expect("stop indexer");
    running.wait().expect("indexer stopped");
    let mut running = indexer(&bus, root);
    let until = Instant::now() + Duration::from_secs(5);
    let mut answer = None;
    while Instant::now() < until && answer.is_none() {
        answer = find(&connection, "indigo");
        sleep(Duration::from_millis(5));
    }
    let first: Vec<String> = answer
        .unwrap_or_default()
        .into_iter()
        .map(|found| found.target)
        .collect();
    assert_eq!(
        first,
        vec![moved.join("note.txt").to_string_lossy().into_owned()],
        "the first answer after a restart already knows the contents"
    );
    assert!(appears(
        &connection,
        "vermilion",
        Path::new("mid:42@example")
    ));
    let cursor: String = connection
        .call_method(
            Some(BUS_NAME),
            OBJECT_PATH,
            Some("org.bande.Search"),
            "Cursor",
            &("org.bande.Mail",),
        )
        .expect("cursor")
        .body()
        .deserialize()
        .expect("cursor text");
    assert_eq!(cursor, "42");

    fs::remove_file(moved.join("note.txt")).expect("delete");
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until && !targets(&connection, "indigo").is_empty() {
        sleep(Duration::from_millis(100));
    }
    assert!(
        targets(&connection, "indigo").is_empty(),
        "a deleted file leaves the results"
    );
    running.kill().expect("stop indexer");
    running.wait().expect("indexer stopped");
}
