#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use unbound_xdg::{
    DesktopEntry, ExecError, Locale, TERMINAL, UserDir, applications, find_icon, launch, user_dir,
};

fn desktops() -> Vec<String> {
    vec!["Unbound".to_owned()]
}

fn entry(text: &str) -> Option<DesktopEntry> {
    DesktopEntry::parse(
        "test.desktop",
        Path::new("/usr/share/applications/test.desktop"),
        text,
        None,
        &desktops(),
    )
}

fn app(exec: &str) -> DesktopEntry {
    entry(&format!(
        "[Desktop Entry]\nType=Application\nName=Test App\nIcon=test-icon\nExec={exec}\n"
    ))
    .expect("a shown application")
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("directories");
    fs::write(path, text).expect("write");
}

#[test]
fn quoted_exec_arguments_follow_the_specifications_escapes() {
    let entry = app(r#""/opt/my app/bin" --flag "a \\"quoted\\" arg" "\\\\" %U"#);

    assert_eq!(
        entry.command(),
        Ok(vec![
            "/opt/my app/bin".to_owned(),
            "--flag".to_owned(),
            "a \"quoted\" arg".to_owned(),
            "\\".to_owned(),
        ])
    );
}

#[test]
fn field_codes_expand_to_the_icon_name_and_location() {
    let entry = app("viewer %i --title=%c %k 100%% %f");

    assert_eq!(
        entry.command(),
        Ok(vec![
            "viewer".to_owned(),
            "--icon".to_owned(),
            "test-icon".to_owned(),
            "--title=Test App".to_owned(),
            "/usr/share/applications/test.desktop".to_owned(),
            "100%".to_owned(),
        ])
    );
}

#[test]
fn reserved_characters_must_be_quoted() {
    assert_eq!(
        app("sh -c echo $HOME").command(),
        Err(ExecError::ReservedCharacter('$'))
    );
    assert_eq!(
        app("sh -c \"echo \\\\$HOME\"").command(),
        Ok(vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "echo $HOME".to_owned()
        ])
    );
}

#[test]
fn terminal_apps_run_inside_the_terminal() {
    let entry = entry("[Desktop Entry]\nType=Application\nName=Top\nExec=top\nTerminal=true\n")
        .expect("a shown application");

    assert_eq!(
        entry.command(),
        Ok(vec![TERMINAL.to_owned(), "-e".to_owned(), "top".to_owned()])
    );
}

#[test]
fn localized_names_are_matched_from_the_most_to_the_least_specific_locale() {
    let text = "[Desktop Entry]\nType=Application\nExec=x\nName=Default\nName[sr]=Serbian\nName[sr@Latn]=Serbian Latin\nName[sr_YU]=Yugoslav\n";
    let name = |locale: &str| {
        DesktopEntry::parse(
            "x.desktop",
            Path::new("x.desktop"),
            text,
            Locale::parse(locale).as_ref(),
            &desktops(),
        )
        .expect("shown")
        .name
    };

    assert_eq!(name("sr_YU@Latn"), "Yugoslav");
    assert_eq!(name("sr_CS@Latn"), "Serbian Latin");
    assert_eq!(name("sr_CS.UTF-8"), "Serbian");
    assert_eq!(name("de_DE.UTF-8"), "Default");
    assert_eq!(name("C"), "Default");
}

#[test]
fn desktop_actions_are_listed_in_order_localised_and_run_with_their_own_exec() {
    let text = "[Desktop Entry]\nType=Application\nName=Browser\nIcon=browser\n\
                Exec=browser %u\nActions=private;new-window;broken;missing;\n\n\
                [Desktop Action new-window]\nName=New Window\nName[de]=Neues Fenster\n\
                Exec=browser --new-window\n\n\
                [Desktop Action private]\nName=New Private Window\nIcon=private\n\
                Exec=browser --private --title=%c\n\n\
                [Desktop Action broken]\nName=No Command\n";
    let entry = DesktopEntry::parse(
        "browser.desktop",
        Path::new("browser.desktop"),
        text,
        Locale::parse("de_DE.UTF-8").as_ref(),
        &desktops(),
    )
    .expect("shown");

    let actions: Vec<(&str, &str, Option<&str>)> = entry
        .actions
        .iter()
        .map(|action| {
            (
                action.id.as_str(),
                action.name.as_str(),
                action.icon.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        actions,
        [
            ("private", "New Private Window", Some("private")),
            ("new-window", "Neues Fenster", None),
        ]
    );
    assert_eq!(
        entry.action_command(&entry.actions[0]),
        Ok(vec![
            "browser".to_owned(),
            "--private".to_owned(),
            "--title=Browser".to_owned()
        ])
    );
    assert_eq!(
        entry.action_command(&entry.actions[1]),
        Ok(vec!["browser".to_owned(), "--new-window".to_owned()])
    );
}

#[test]
fn hidden_entries_and_entries_for_other_desktops_are_not_listed() {
    let base = "[Desktop Entry]\nType=Application\nName=A\nExec=a\n";
    assert!(entry(&format!("{base}NoDisplay=true\n")).is_none());
    assert!(entry(&format!("{base}Hidden=true\n")).is_none());
    assert!(entry(&format!("{base}OnlyShowIn=GNOME;KDE;\n")).is_none());
    assert!(entry(&format!("{base}NotShowIn=Unbound;\n")).is_none());
    assert!(entry(&format!("{base}OnlyShowIn=GNOME;Unbound;\n")).is_some());
    assert!(entry(&format!("{base}TryExec=/nonexistent/unbound-app\n")).is_none());
    assert!(entry("[Desktop Entry]\nType=Link\nName=L\nURL=https://example.com\n").is_none());
}

#[test]
fn ids_come_from_the_path_below_applications_and_earlier_directories_win() {
    let home = TempDir::new().expect("home");
    let system = TempDir::new().expect("system");
    write(
        &system.path().join("applications/vendor/editor.desktop"),
        "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor\n",
    );
    write(
        &system.path().join("applications/viewer.desktop"),
        "[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer\n",
    );
    write(
        &home.path().join("applications/viewer.desktop"),
        "[Desktop Entry]\nType=Application\nName=Viewer\nExec=viewer\nHidden=true\n",
    );

    let found = applications(
        &[home.path().to_path_buf(), system.path().to_path_buf()],
        None,
        &desktops(),
    );

    let ids = found
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["vendor-editor.desktop"]);
    assert_eq!(found[0].app_id(), "vendor-editor");
}

const HICOLOR: &str = "[Icon Theme]\nName=Hicolor\nDirectories=48x48/apps,scalable/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n\n[scalable/apps]\nSize=128\nMinSize=8\nMaxSize=512\nType=Scalable\n";

const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##;

fn icon_data() -> (TempDir, Vec<PathBuf>) {
    let data = TempDir::new().expect("data dir");
    let icons = data.path().join("icons");
    write(&icons.join("hicolor/index.theme"), HICOLOR);
    write(
        &icons.join("Fancy/index.theme"),
        "[Icon Theme]\nName=Fancy\nInherits=hicolor\nDirectories=\n",
    );
    let png = image::RgbaImage::from_pixel(48, 48, image::Rgba([0, 0, 255, 255]));
    fs::create_dir_all(icons.join("hicolor/48x48/apps")).expect("directories");
    png.save(icons.join("hicolor/48x48/apps/editor.png"))
        .expect("png");
    write(&icons.join("hicolor/scalable/apps/editor.svg"), SQUARE);
    write(&data.path().join("pixmaps/legacy.svg"), SQUARE);
    let dirs = vec![data.path().to_path_buf()];
    (data, dirs)
}

#[test]
fn icons_are_found_by_size_through_inherited_themes() {
    let (data, dirs) = icon_data();
    let icons = data.path().join("icons/hicolor");

    assert_eq!(
        find_icon("editor", 48, &["Fancy"], &dirs),
        Some(icons.join("48x48/apps/editor.png"))
    );
    assert_eq!(
        find_icon("editor", 64, &["Fancy"], &dirs),
        Some(icons.join("scalable/apps/editor.svg"))
    );
    assert_eq!(
        find_icon("legacy", 48, &["Fancy"], &dirs),
        Some(data.path().join("pixmaps/legacy.svg"))
    );
    assert_eq!(find_icon("missing", 48, &["Fancy"], &dirs), None);
}

#[test]
fn launched_apps_receive_the_activation_token() {
    let dir = TempDir::new().expect("dir");
    let output = dir.path().join("token");
    let entry = app(&format!(
        "sh -c \"echo \\\\$XDG_ACTIVATION_TOKEN > {}\"",
        output.display()
    ));

    launch(&entry, Some("token-123")).expect("launch");

    let deadline = Instant::now() + Duration::from_secs(10);
    while fs::read_to_string(&output).map_or(true, |text| text.trim().is_empty()) {
        assert!(Instant::now() < deadline, "the app did not run");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(&output).expect("token").trim(),
        "token-123"
    );
}

#[test]
fn user_dirs_default_to_their_english_names_in_home() {
    let home = Path::new("/home/ada");

    assert_eq!(user_dir(home, None, UserDir::Desktop), home.join("Desktop"));
    assert_eq!(
        user_dir(home, Some("# nothing\n"), UserDir::Downloads),
        home.join("Downloads")
    );
}

#[test]
fn user_dirs_follow_xdg_user_dirs() {
    let home = Path::new("/home/ada");
    let user_dirs = "# written by xdg-user-dirs-update\n\
        XDG_DOWNLOAD_DIR=\"$HOME/Hämtningar\"\n\
        XDG_DESKTOP_DIR=\"$HOME/Skrivbord\"\n";

    assert_eq!(
        user_dir(home, Some(user_dirs), UserDir::Desktop),
        home.join("Skrivbord")
    );
    assert_eq!(
        user_dir(home, Some(user_dirs), UserDir::Downloads),
        home.join("Hämtningar")
    );
    assert_eq!(
        user_dir(
            home,
            Some("XDG_DESKTOP_DIR=\"/srv/desk\""),
            UserDir::Desktop
        ),
        PathBuf::from("/srv/desk")
    );
    assert_eq!(
        user_dir(home, Some("XDG_DESKTOP_DIR=\"$HOME/\""), UserDir::Desktop),
        home.to_path_buf()
    );
}

#[test]
fn a_relative_or_unquoted_user_dir_is_ignored() {
    let home = Path::new("/home/ada");

    assert_eq!(
        user_dir(home, Some("XDG_DESKTOP_DIR=\"desk\""), UserDir::Desktop),
        home.join("Desktop")
    );
    assert_eq!(
        user_dir(home, Some("XDG_DESKTOP_DIR=$HOME/desk"), UserDir::Desktop),
        home.join("Desktop")
    );
}
