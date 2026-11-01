#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bande_xdg::{
    DesktopEntry, ExecError, ICON_THEMES, Locale, TERMINAL, UserDir, app_name, applications,
    apps_to_reopen, autostart_dir, find_app, find_app_for_process, find_icon, launch,
    opens_at_login, remember_apps, running_app, set_aside_apps_of_unclean_end, set_open_at_login,
    take_apps_to_reopen, user_dir, write_launcher, write_running_apps,
};
use tempfile::TempDir;

fn desktops() -> Vec<String> {
    vec!["Bande".to_owned()]
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
    assert!(entry(&format!("{base}NotShowIn=Bande;\n")).is_none());
    assert!(entry(&format!("{base}OnlyShowIn=GNOME;Bande;\n")).is_some());
    assert!(entry(&format!("{base}TryExec=/nonexistent/bande-app\n")).is_none());
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
fn the_bande_theme_draws_its_own_icons_and_adwaita_the_rest() {
    let system = TempDir::new().expect("data dir");
    let adwaita = system.path().join("icons/Adwaita");
    write(
        &adwaita.join("index.theme"),
        &HICOLOR
            .replace("48x48/apps,scalable/apps", "scalable/apps")
            .replace("Name=Hicolor", "Name=Adwaita"),
    );
    for name in ["folder", "user-trash", "text-x-generic", "computer"] {
        write(&adwaita.join(format!("scalable/apps/{name}.svg")), SQUARE);
    }
    let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data");
    let dirs = vec![system.path().to_path_buf(), shipped.clone()];
    let ours = shipped.join("icons/Bande");

    for (name, directory) in [
        ("folder", "scalable/places"),
        ("user-trash", "scalable/places"),
        ("user-trash-full", "scalable/places"),
        ("text-x-generic", "scalable/mimetypes"),
        ("application-pdf", "scalable/mimetypes"),
        ("org.bande.Files", "scalable/apps"),
        ("org.gnome.Papers", "scalable/apps"),
        ("network-wireless-signal-good-symbolic", "symbolic/status"),
        ("go-previous-symbolic", "symbolic/actions"),
    ] {
        assert_eq!(
            find_icon(name, 32, &ICON_THEMES, &dirs),
            Some(ours.join(format!("{directory}/{name}.svg")))
        );
    }
    assert_eq!(
        find_icon("computer", 32, &ICON_THEMES, &dirs),
        Some(adwaita.join("scalable/apps/computer.svg"))
    );
    assert_eq!(
        find_icon("folder", 32, &["Bande-Teal", "Bande", "Adwaita"], &dirs),
        Some(shipped.join("icons/Bande-Teal/scalable/places/folder.svg"))
    );
    assert_eq!(
        find_icon("user-trash", 32, &["Bande-Teal", "Bande", "Adwaita"], &dirs),
        Some(ours.join("scalable/places/user-trash.svg"))
    );
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
fn user_dirs_follow_xdg_user_dirs_and_default_to_english_names_in_home() {
    let home = Path::new("/home/ada");
    let user_dirs = "# written by xdg-user-dirs-update\n\
        XDG_DOWNLOAD_DIR=\"$HOME/Hämtningar\"\n\
        XDG_DESKTOP_DIR=\"$HOME/Skrivbord\"\n";

    for (file, dir, expected) in [
        (None, UserDir::Desktop, home.join("Desktop")),
        (
            Some("# nothing\n"),
            UserDir::Downloads,
            home.join("Downloads"),
        ),
        (Some(user_dirs), UserDir::Desktop, home.join("Skrivbord")),
        (Some(user_dirs), UserDir::Downloads, home.join("Hämtningar")),
        (
            Some("XDG_DESKTOP_DIR=\"/srv/desk\""),
            UserDir::Desktop,
            PathBuf::from("/srv/desk"),
        ),
        (
            Some("XDG_DESKTOP_DIR=\"$HOME/\""),
            UserDir::Desktop,
            home.to_path_buf(),
        ),
        (
            Some("XDG_DESKTOP_DIR=\"desk\""),
            UserDir::Desktop,
            home.join("Desktop"),
        ),
        (
            Some("XDG_DESKTOP_DIR=$HOME/desk"),
            UserDir::Desktop,
            home.join("Desktop"),
        ),
    ] {
        assert_eq!(user_dir(home, file, dir), expected, "{file:?}");
    }
}

#[test]
fn open_at_login_copies_the_desktop_entry_into_autostart_and_removes_it_again() {
    let home = tempfile::tempdir().expect("home");
    let dir = autostart_dir(None, Some(home.path())).expect("autostart folder");
    let entry = home.path().join("org.example.Editor.desktop");
    std::fs::write(&entry, "[Desktop Entry]\nName=Editor\nExec=editor\n").expect("entry");
    let id = "org.example.Editor.desktop";

    assert!(!opens_at_login(&dir, id));
    set_open_at_login(&dir, id, &entry, true).expect("open at login");
    assert!(opens_at_login(&dir, id));
    std::fs::write(dir.join(id), "[Desktop Entry]\nHidden=true\n").expect("hidden");
    assert!(!opens_at_login(&dir, id));
    set_open_at_login(&dir, id, &entry, false).expect("stop opening at login");
    assert!(!dir.join(id).exists());
    set_open_at_login(&dir, id, &entry, false).expect("already off");
}

fn installed(id: &str, keys: &str) -> DesktopEntry {
    DesktopEntry::parse(
        &format!("{id}.desktop"),
        &Path::new("/usr/share/applications").join(format!("{id}.desktop")),
        &format!("[Desktop Entry]\nType=Application\nName={id}\n{keys}\n"),
        None,
        &desktops(),
    )
    .expect("a shown application")
}

#[test]
fn window_app_ids_find_their_desktop_entry() {
    let entries = [
        installed("org.gimp.GIMP", "Exec=gimp-3.0 %U"),
        installed("slack", "Exec=/usr/bin/slack %U"),
        installed(
            "signal-desktop",
            "Exec=/opt/Signal/signal-desktop --no-sandbox %U\nStartupWMClass=Signal",
        ),
        installed("firefox", "Exec=firefox %u"),
        installed("visual-studio-code", "Exec=/usr/share/code/code %F"),
        installed("org.gnome.Terminal", "Exec=gnome-terminal"),
        installed("org.bande.Files", "Exec=bande-files"),
        installed("org.bande.files", "Exec=sh -c files"),
        installed("python-tool", "Exec=python3 tool.py"),
    ];
    let found = |app_id: &str| find_app(&entries, app_id).map(DesktopEntry::app_id);

    assert_eq!(found("gimp"), Some("org.gimp.GIMP"));
    assert_eq!(found("Slack"), Some("slack"));
    assert_eq!(found("signal"), Some("signal-desktop"));
    assert_eq!(found("org.mozilla.firefox"), Some("firefox"));
    assert_eq!(found("code"), Some("visual-studio-code"));
    assert_eq!(found("org.bande.files"), Some("org.bande.files"));
    assert_eq!(found("ORG.BANDE.FILES"), Some("org.bande.Files"));
    assert_eq!(found("org.example.Terminal"), None);
    assert_eq!(found("python3"), None);
    assert_eq!(found(""), None);

    assert_eq!(app_name(&entries, "signal"), "signal-desktop");
    assert_eq!(app_name(&entries, "org.example.Unknown"), "Unknown");
    assert_eq!(app_name(&entries, ""), "");

    let running = ["gimp".to_owned(), "org.example.Unknown".to_owned()];
    assert_eq!(
        running_app(&entries, &running, "org.gimp.GIMP"),
        Some("gimp")
    );
    assert_eq!(
        running_app(&entries, &running, "org.example.Unknown"),
        Some("org.example.Unknown")
    );
    assert_eq!(running_app(&entries, &running, "slack"), None);
}

fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    write(path, "");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("executable");
}

fn process(root: &Path, pid: u32, unit: &str, exe: &Path) {
    let folder = root.join(pid.to_string());
    write(
        &folder.join("cgroup"),
        &format!("0::/user.slice/user-1000.slice/user@1000.service/app.slice/{unit}\n"),
    );
    std::os::unix::fs::symlink(exe, folder.join("exe")).expect("exe link");
}

#[test]
fn a_window_finds_its_app_through_its_own_scope_or_its_program() {
    let dir = TempDir::new().expect("dir");
    let bin = dir.path().join("bin");
    let (editor, python) = (bin.join("my editor"), bin.join("python3"));
    executable(&editor);
    executable(&python);
    let entries = [
        installed("org.bande.Terminal", "Exec=true"),
        installed("com.example.Editor", "Exec=true"),
        installed("org.example.Paint", "Exec=true"),
        installed(
            "org.example.Writer",
            &format!("Exec=\"{}\" %F", editor.display()),
        ),
        installed(
            "org.example.Tool",
            &format!("Exec={} tool.py", python.display()),
        ),
    ];
    let root = dir.path().join("proc");
    process(&root, 42, "app-com.example.Editor-42.scope", &python);
    process(&root, 43, "app-bande-org.bande.Terminal-7.scope", &editor);
    process(&root, 44, "app-bande-org.bande.Terminal-7.scope", &python);
    process(&root, 45, "app-flatpak-org.example.Paint-99.scope", &python);
    let found = |pid| find_app_for_process(&entries, &root, pid).map(DesktopEntry::app_id);

    assert_eq!(found(42), Some("com.example.Editor"));
    assert_eq!(found(43), Some("org.example.Writer"));
    assert_eq!(found(44), None);
    assert_eq!(found(45), Some("org.example.Paint"));
    assert_eq!(found(46), None);
}

#[test]
fn a_kept_app_without_an_entry_gets_a_launcher_that_starts_its_program() {
    let dir = TempDir::new().expect("dir");
    let bin = dir.path().join("my bin");
    let (editor, python) = (bin.join("dev \"build\""), bin.join("python3"));
    executable(&editor);
    executable(&python);
    let root = dir.path().join("proc");
    process(&root, 50, "app-bande-org.bande.Terminal-7.scope", &editor);
    process(&root, 51, "app-bande-org.bande.Terminal-7.scope", &python);
    write(&root.join("51/cmdline"), "python3\0-u\0tool.py\0--fast\0");
    std::os::unix::fs::symlink(&bin, root.join("51/cwd")).expect("cwd link");
    let data_home = dir.path().join("data");

    let written = write_launcher(&data_home, "org.example.Stray", &root, 50).expect("a launcher");
    write_launcher(&data_home, "org.example.Script", &root, 51).expect("a launcher");
    assert_eq!(
        written,
        data_home.join("applications/org.example.Stray.desktop")
    );
    let entries = applications(std::slice::from_ref(&data_home), None, &desktops());
    let stray = find_app(&entries, "org.example.Stray").expect("the written entry");
    assert_eq!(stray.name, "Stray");
    assert_eq!(stray.command(), Ok(vec![editor.display().to_string()]));
    let script = find_app(&entries, "org.example.Script").expect("the written entry");
    assert_eq!(
        script.command(),
        Ok(vec![
            python.display().to_string(),
            bin.join("tool.py").display().to_string()
        ])
    );

    let before = fs::read_to_string(&written).expect("launcher");
    assert!(write_launcher(&data_home, "org.example.Stray", &root, 51).is_err());
    assert_eq!(fs::read_to_string(&written).expect("launcher"), before);
    assert!(write_launcher(&data_home, "org.example.Gone", &root, 99).is_err());
}

#[test]
fn running_apps_are_remembered_once_by_desktop_id_and_reopened_only_once() {
    let entries = [
        installed("firefox", "Exec=firefox %u"),
        installed("org.bande.Files", "Exec=bande-files"),
    ];
    let running: Vec<String> = ["firefox", "files", "firefox", "org.bande.MenuBar"]
        .map(str::to_owned)
        .to_vec();
    let apps = apps_to_reopen(&entries, &running);
    assert_eq!(apps, ["firefox", "org.bande.Files"]);

    let state = TempDir::new().expect("temporary directory");
    let file = state.path().join("bande/reopen");
    remember_apps(&file, &apps).expect("remember");
    assert_eq!(take_apps_to_reopen(&file), ["firefox", "org.bande.Files"]);
    assert!(!file.exists());
    assert!(take_apps_to_reopen(&file).is_empty());
}

#[test]
fn the_running_apps_are_set_aside_only_when_the_session_ended_without_remembering() {
    let state = TempDir::new().expect("temporary directory");
    let running = state.path().join("bande/running");
    let reopen = state.path().join("bande/reopen");
    let crashed = state.path().join("bande/crashed");
    let apps = ["firefox".to_owned(), "files".to_owned()];

    write_running_apps(&running, &apps).expect("write");
    set_aside_apps_of_unclean_end(&running, &reopen, &crashed).expect("set aside");
    assert!(!running.exists());
    assert_eq!(take_apps_to_reopen(&crashed), apps);

    write_running_apps(&running, &apps).expect("write");
    remember_apps(&reopen, &["firefox"]).expect("remember");
    set_aside_apps_of_unclean_end(&running, &reopen, &crashed).expect("set aside");
    assert!(!running.exists());
    assert!(!crashed.exists());
    assert_eq!(take_apps_to_reopen(&reopen), ["firefox"]);

    set_aside_apps_of_unclean_end(&running, &reopen, &crashed).expect("nothing to set aside");
    assert!(!crashed.exists());
}
