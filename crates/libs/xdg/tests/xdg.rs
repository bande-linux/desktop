#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use bande_xdg::{
    Apps, DesktopEntry, ExecError, listed_apps, reopening_marker, saved_windows_file,
    set_aside_apps_of_unclean_end, take_apps_to_reopen,
};
use tempfile::TempDir;

fn app(exec: &str) -> DesktopEntry {
    DesktopEntry::parse(
        "test.desktop",
        Path::new("/usr/share/applications/test.desktop"),
        &format!("[Desktop Entry]\nType=Application\nName=Test App\nIcon=test-icon\nExec={exec}\n"),
        None,
        &["X-Bande".to_owned()],
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

struct LastSession {
    _state: TempDir,
    running: PathBuf,
    reopen: PathBuf,
    crashed: PathBuf,
}

impl LastSession {
    fn new() -> Self {
        let state = TempDir::new().expect("state");
        Self {
            running: state.path().join("running"),
            reopen: state.path().join("reopen"),
            crashed: state.path().join("crashed"),
            _state: state,
        }
    }

    fn ended(&self, cleanly: bool) {
        fs::write(&self.running, "org.bande.Files\n").expect("running");
        if cleanly {
            fs::write(&self.reopen, "org.bande.Files\n").expect("reopen");
        }
    }

    fn next_login(&self) -> Vec<String> {
        set_aside_apps_of_unclean_end(&self.running, &self.reopen, &self.crashed)
            .expect("set aside");
        assert!(!self.running.exists());
        take_apps_to_reopen(&self.reopen);
        listed_apps(&self.crashed)
    }
}

#[test]
fn only_a_crash_asks_to_reopen_and_only_once() {
    let session = LastSession::new();
    session.ended(true);
    assert!(session.next_login().is_empty());

    session.ended(false);
    assert_eq!(session.next_login(), ["org.bande.Files"]);
    assert!(
        session.next_login().is_empty(),
        "an unanswered question is not asked again"
    );

    session.ended(true);
    assert!(session.next_login().is_empty());
}

#[test]
fn each_app_notes_its_own_reopening() {
    let viewer = saved_windows_file("org.bande.Viewer").expect("state");
    let files = saved_windows_file("org.bande.Files").expect("state");
    assert_ne!(reopening_marker(&viewer), reopening_marker(&files));
}
