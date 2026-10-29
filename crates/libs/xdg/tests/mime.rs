#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use unbound_xdg::{
    DesktopEntry, Locale, MimeApps, MimeDatabase, file_uri, handlers, mimeapps_paths,
    path_from_file_uri, set_default_app,
};

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().expect("parent")).expect("directories");
    fs::write(path, bytes).expect("write");
}

fn magic_section(priority: u32, mime: &str, lines: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = format!("[{priority}:{mime}]\n").into_bytes();
    for line in lines {
        bytes.extend(line);
        bytes.push(b'\n');
    }
    bytes
}

fn matchlet(indent: &str, offset: usize, value: &[u8], suffix: &[u8]) -> Vec<u8> {
    let mut bytes = format!("{indent}>{offset}=").into_bytes();
    bytes.extend((value.len() as u16).to_be_bytes());
    bytes.extend(value);
    bytes.extend(suffix);
    bytes
}

fn database() -> (TempDir, TempDir, MimeDatabase) {
    let user = TempDir::new().expect("user");
    let system = TempDir::new().expect("system");
    let mime = system.path().join("mime");
    write(
        &mime.join("globs2"),
        "# generated\n\
         50:text/x-csrc:*.c:cs\n\
         50:text/x-c++src:*.C:cs\n\
         50:image/png:*.png\n\
         50:application/x-compressed-tar:*.tar.gz\n\
         50:application/gzip:*.gz\n\
         60:text/x-makefile:makefile\n\
         50:application/x-sharedlib:*.so.[0-9]*\n\
         50:video/mp2t:*.ts\n\
         50:text/vnd.qt.linguist:*.ts\n\
         50:application/x-old:*.old\n\
         40:text/x-readme:README*\n",
    );
    let mut magic = b"MIME-Magic\0\n".to_vec();
    magic.extend(magic_section(
        50,
        "image/png",
        &[matchlet("", 0, b"\x89PNG", b"")],
    ));
    magic.extend(magic_section(
        50,
        "application/x-wrapped",
        &[
            matchlet("", 0, b"WRAP", b""),
            matchlet("1", 8, b"inner", b"+16"),
        ],
    ));
    magic.extend(magic_section(
        40,
        "application/x-masked",
        &[matchlet("", 2, b"\xF0\x0F", b"&\xF0\x0F")],
    ));
    magic.extend(magic_section(
        50,
        "text/vnd.qt.linguist",
        &[matchlet("", 0, b"<?xml", b"")],
    ));
    magic.extend(magic_section(
        30,
        "application/x-word",
        &[matchlet("", 0, &[0x34, 0x12], b"~2")],
    ));
    write(&mime.join("magic"), magic);
    write(
        &mime.join("subclasses"),
        "text/x-c++src text/x-csrc\napplication/x-compressed-tar application/gzip\n",
    );
    write(&mime.join("aliases"), "image/x-png image/png\n");
    write(
        &mime.join("generic-icons"),
        "application/x-compressed-tar:package-x-generic\n",
    );
    write(&mime.join("icons"), "text/x-csrc:source-c\n");
    write(
        &mime.join("image/png.xml"),
        "<?xml version=\"1.0\"?>\n<mime-type type=\"image/png\">\n  <comment>PNG image</comment>\n  <comment xml:lang=\"de\">PNG-Bild &amp; mehr</comment>\n</mime-type>\n",
    );
    write(
        &user.path().join("mime/globs2"),
        "50:application/x-old:__NOGLOBS__\n50:text/x-new:*.new\n",
    );
    let database = MimeDatabase::load(&[user.path().to_path_buf(), system.path().to_path_buf()]);
    (user, system, database)
}

#[test]
fn names_are_matched_by_literal_then_longest_glob_with_case_rules() {
    let (_user, _system, database) = database();

    assert_eq!(database.for_name("hello.c"), Some("text/x-csrc"));
    assert_eq!(database.for_name("Hello.C"), Some("text/x-c++src"));
    assert_eq!(database.for_name("PHOTO.PNG"), Some("image/png"));
    assert_eq!(
        database.for_name("backup.tar.gz"),
        Some("application/x-compressed-tar")
    );
    assert_eq!(database.for_name("notes.gz"), Some("application/gzip"));
    assert_eq!(database.for_name("Makefile"), Some("text/x-makefile"));
    assert_eq!(
        database.for_name("libfoo.so.6"),
        Some("application/x-sharedlib")
    );
    assert_eq!(database.for_name("README.md"), Some("text/x-readme"));
    assert_eq!(database.for_name("é.PNG"), Some("image/png"));
    assert_eq!(database.for_name("plain"), None);
}

#[test]
fn a_higher_directory_replaces_and_extends_the_globs_below() {
    let (_user, _system, database) = database();

    assert_eq!(database.for_name("file.old"), None);
    assert_eq!(database.for_name("file.new"), Some("text/x-new"));
}

#[test]
fn contents_decide_when_the_name_says_nothing_or_is_ambiguous() {
    let (_user, _system, database) = database();
    let files = TempDir::new().expect("files");
    let file = |name: &str, bytes: &[u8]| {
        let path = files.path().join(name);
        fs::write(&path, bytes).expect("write");
        path
    };

    assert_eq!(
        database.for_path(&file("picture", b"\x89PNG\r\n")),
        "image/png"
    );
    assert_eq!(
        database.for_path(&file("subtitles.ts", b"<?xml version")),
        "text/vnd.qt.linguist"
    );
    assert_eq!(
        database.for_path(&file("wrapped", b"WRAP\0\0\0\0\0\0\0inner")),
        "application/x-wrapped"
    );
    assert_eq!(
        database.for_path(&file("unwrapped", b"WRAP\0\0\0\0\0\0\0outer")),
        "application/octet-stream"
    );
    assert_eq!(
        database.for_path(&file("masked", b"\0\0\xFF\xFF")),
        "application/x-masked"
    );
    assert_eq!(
        database.for_path(&file("word", &0x3412u16.to_ne_bytes())),
        "application/x-word"
    );
    assert_eq!(
        database.for_path(&file("notes", "plain text, ünïcode".as_bytes())),
        "text/plain"
    );
    assert_eq!(database.for_path(&file("empty", b"")), "text/plain");
    assert_eq!(
        database.for_path(&file("binary", b"\x01\x02\0\x03")),
        "application/octet-stream"
    );
    assert_eq!(database.for_path(files.path()), "inode/directory");
}

#[test]
fn ancestry_follows_aliases_subclasses_and_text() {
    let (_user, _system, database) = database();

    assert_eq!(
        database.ancestors("text/x-c++src"),
        ["text/x-c++src", "text/x-csrc", "text/plain"]
    );
    assert_eq!(database.ancestors("image/x-png"), ["image/png"]);
    assert!(database.is_a("application/x-compressed-tar", "application/gzip"));
    assert!(!database.is_a("application/gzip", "application/x-compressed-tar"));
}

#[test]
fn icons_fall_back_from_specific_to_generic_names() {
    let (_user, _system, database) = database();

    assert_eq!(
        database.icon_names("image/x-png"),
        ["image-png", "image-x-generic", "application-x-generic"]
    );
    assert_eq!(
        database.icon_names("application/x-compressed-tar"),
        [
            "application-x-compressed-tar",
            "package-x-generic",
            "application-x-generic"
        ]
    );
    assert_eq!(
        database.icon_names("text/x-csrc"),
        ["source-c", "text-x-generic", "application-x-generic"]
    );
}

#[test]
fn descriptions_are_localised() {
    let (_user, _system, database) = database();
    let german = Locale::parse("de_DE.UTF-8");

    assert_eq!(
        database.description("image/x-png", None).as_deref(),
        Some("PNG image")
    );
    assert_eq!(
        database
            .description("image/png", german.as_ref())
            .as_deref(),
        Some("PNG-Bild & mehr")
    );
    assert_eq!(database.description("text/x-csrc", None), None);
}

fn handler_data() -> (TempDir, Vec<DesktopEntry>) {
    let data = TempDir::new().expect("data");
    let app = |name: &str, extra: &str| {
        write(
            &data.path().join(format!("applications/{name}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name} %F\n{extra}"),
        );
    };
    app("viewer", "MimeType=image/png;\n");
    app("painter", "MimeType=image/png;\n");
    app("editor", "MimeType=text/plain;\n");
    app("helper", "MimeType=image/png;\nNoDisplay=true\n");
    app("archiver", "");
    app("gone", "MimeType=image/png;\nHidden=true\n");
    let found = handlers(&[data.path().to_path_buf()], None, &["Unbound".to_owned()]);
    (data, found)
}

fn ids(entries: &[&DesktopEntry]) -> Vec<String> {
    entries.iter().map(|entry| entry.id.clone()).collect()
}

fn ancestry(types: &[&str]) -> Vec<String> {
    types.iter().map(|mime| (*mime).to_owned()).collect()
}

#[test]
fn handlers_include_entries_hidden_from_menus() {
    let (_data, found) = handler_data();

    let names = found
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "archiver.desktop",
            "editor.desktop",
            "helper.desktop",
            "painter.desktop",
            "viewer.desktop"
        ]
    );
}

#[test]
fn defaults_follow_list_precedence_and_associations() {
    let (_data, found) = handler_data();
    let lists = TempDir::new().expect("lists");
    let config = lists.path().join("config");
    let system = lists.path().join("system");
    write(
        &config.join("unbound-mimeapps.list"),
        "[Default Applications]\nimage/png=missing.desktop;painter.desktop;\n",
    );
    write(
        &config.join("mimeapps.list"),
        "[Default Applications]\nimage/png=viewer.desktop\n\n[Added Associations]\nimage/png=archiver.desktop;\n\n[Removed Associations]\nimage/png=helper.desktop;\n",
    );
    write(
        &system.join("applications/mimeapps.list"),
        "[Added Associations]\nimage/png=helper.desktop;\ntext/x-csrc=archiver.desktop;\n",
    );
    let paths = mimeapps_paths(&[config], &[system], &["Unbound".to_owned()]);
    let apps = MimeApps::load(&paths);
    let png = ancestry(&["image/png"]);

    assert_eq!(
        apps.default_for(&png, &found)
            .map(|entry| entry.id.as_str()),
        Some("painter.desktop")
    );
    assert_eq!(
        ids(&apps.apps_for(&png, &found)),
        ["painter.desktop", "archiver.desktop", "viewer.desktop"]
    );
    let c = ancestry(&["text/x-csrc", "text/plain"]);
    assert_eq!(
        apps.default_for(&c, &found).map(|entry| entry.id.as_str()),
        Some("archiver.desktop")
    );
    assert_eq!(
        ids(&apps.apps_for(&c, &found)),
        ["archiver.desktop", "editor.desktop"]
    );
    assert_eq!(
        apps.default_for(&ancestry(&["text/markdown", "text/plain"]), &found)
            .map(|entry| entry.id.as_str()),
        Some("editor.desktop")
    );
    assert!(
        apps.default_for(&ancestry(&["video/mp4"]), &found)
            .is_none()
    );
}

#[test]
fn choosing_a_default_app_rewrites_only_its_keys() {
    let (_data, found) = handler_data();
    let dir = TempDir::new().expect("dir");
    let list = dir.path().join("mimeapps.list");
    write(
        &list,
        "# mine\n[Default Applications]\ntext/plain=editor.desktop\nimage/png=viewer.desktop\n\n[Added Associations]\nimage/png=viewer.desktop;archiver.desktop;\n",
    );

    set_default_app(&list, "image/png", "archiver.desktop").expect("set default");
    set_default_app(&list, "video/mp4", "viewer.desktop").expect("set default");

    let text = fs::read_to_string(&list).expect("read");
    assert!(text.starts_with("# mine\n"));
    assert!(text.contains("text/plain=editor.desktop\n"));
    assert!(text.contains("image/png=archiver.desktop;\n"));
    assert!(text.contains("image/png=archiver.desktop;viewer.desktop;\n"));
    let apps = MimeApps::load(&[list]);
    assert_eq!(
        apps.default_for(&ancestry(&["video/mp4"]), &found)
            .map(|entry| entry.id.as_str()),
        Some("viewer.desktop")
    );
    assert_eq!(
        apps.default_for(&ancestry(&["image/png"]), &found)
            .map(|entry| entry.id.as_str()),
        Some("archiver.desktop")
    );
}

fn entry_with(exec: &str) -> DesktopEntry {
    DesktopEntry::parse(
        "app.desktop",
        Path::new("/usr/share/applications/app.desktop"),
        &format!("[Desktop Entry]\nType=Application\nName=App\nExec={exec}\n"),
        None,
        &["Unbound".to_owned()],
    )
    .expect("entry")
}

#[test]
fn files_fill_the_exec_field_codes() {
    let files = vec![PathBuf::from("/home/ada/a b.txt"), PathBuf::from("/tmp/ü")];

    assert_eq!(
        entry_with("edit --new %F")
            .commands_for(&files)
            .expect("exec"),
        [vec!["edit", "--new", "/home/ada/a b.txt", "/tmp/ü"]]
    );
    assert_eq!(
        entry_with("view %f").commands_for(&files).expect("exec"),
        [vec!["view", "/home/ada/a b.txt"], vec!["view", "/tmp/ü"]]
    );
    assert_eq!(
        entry_with("browse %U").commands_for(&files).expect("exec"),
        [vec![
            "browse",
            "file:///home/ada/a%20b.txt",
            "file:///tmp/%C3%BC"
        ]]
    );
    assert_eq!(
        entry_with("game").commands_for(&files).expect("exec"),
        [vec!["game"]]
    );
    assert_eq!(
        entry_with("view %f").commands_for(&[]).expect("exec"),
        [vec!["view"]]
    );
}

#[test]
fn file_uris_round_trip_any_path() {
    let path = Path::new(OsStr::from_bytes(b"/home/ada/50% off \xFF#1"));

    let uri = file_uri(path);

    assert_eq!(uri, "file:///home/ada/50%25%20off%20%FF%231");
    assert_eq!(path_from_file_uri(&uri).as_deref(), Some(path));
    assert_eq!(
        path_from_file_uri("file://localhost/srv/a").as_deref(),
        Some(Path::new("/srv/a"))
    );
    assert_eq!(path_from_file_uri("https://example.com/"), None);
}
