#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use bande_xdg::{Apps, DesktopEntry, ExecError};
use tempfile::TempDir;

fn app(exec: &str) -> DesktopEntry {
    DesktopEntry::parse(
        "test.desktop",
        Path::new("/usr/share/applications/test.desktop"),
        &format!("[Desktop Entry]\nType=Application\nName=Test App\nIcon=test-icon\nExec={exec}\n"),
        None,
        &["Bande".to_owned()],
    )
    .expect("a shown application")
}

#[test]
fn exec_lines_follow_the_quoting_escapes_and_field_codes_of_the_specification() {
    assert_eq!(
        app(r#""/opt/my app/bin" "a \\"quoted\\" arg" "\\\\" %i 100%% %U"#).command(),
        Ok(vec![
            "/opt/my app/bin".to_owned(),
            "a \"quoted\" arg".to_owned(),
            "\\".to_owned(),
            "--icon".to_owned(),
            "test-icon".to_owned(),
            "100%".to_owned(),
        ])
    );
    assert_eq!(
        app("sh -c echo $HOME").command(),
        Err(ExecError::ReservedCharacter('$'))
    );
}

#[test]
fn a_launcher_file_runs_only_when_installed_or_marked_executable_by_its_owner() {
    let data = TempDir::new().expect("data");
    let downloads = TempDir::new().expect("downloads");
    let entry = "[Desktop Entry]\nType=Application\nName=Tool\nExec=tool\n";
    let installed = data.path().join("applications/tool.desktop");
    fs::create_dir_all(installed.parent().expect("folder")).expect("folder");
    fs::write(&installed, entry).expect("installed");
    let downloaded = downloads.path().join("tool.desktop");
    fs::write(&downloaded, entry).expect("downloaded");
    let apps = Apps::load(vec![data.path().to_path_buf()], &[]);
    let mime = "application/x-desktop";

    assert!(apps.launcher(&installed, mime).is_some());
    assert!(apps.launcher(&downloaded, mime).is_none());
    fs::set_permissions(&downloaded, fs::Permissions::from_mode(0o755)).expect("chmod");
    assert!(apps.launcher(&downloaded, mime).is_some());
}
