#![allow(clippy::expect_used)]

use unbound_dock_model::autostart::{autostart_dir, opens_at_login, set_open_at_login};
use unbound_dock_model::bus::{app_id_from_uri, launcher_update};
use unbound_dock_model::{LauncherEntry, arrange};
use zbus::zvariant::{OwnedValue, Value};

#[test]
fn kept_apps_that_are_not_installed_keep_their_place_when_the_apps_are_rearranged() {
    let kept = ["a", "gone", "b", "c"].map(str::to_owned);
    let shown = ["c", "a", "b"].map(str::to_owned);
    assert_eq!(
        arrange(&kept, &shown, |app_id| app_id == "gone"),
        ["c", "a", "gone", "b"]
    );
    assert_eq!(
        arrange(&kept, &["b".to_owned()], |app_id| app_id == "gone"),
        ["gone", "b"]
    );
}

#[test]
fn launcher_entry_updates_merge_as_kde_reads_them() {
    assert_eq!(
        app_id_from_uri("application://org.mozilla.firefox.desktop").as_deref(),
        Some("org.mozilla.firefox")
    );
    let update = |pairs: &[(&str, Value<'static>)]| {
        launcher_update(
            &pairs
                .iter()
                .map(|(key, value)| {
                    (
                        (*key).to_owned(),
                        OwnedValue::try_from(value.clone()).expect("value"),
                    )
                })
                .collect(),
        )
    };
    let mut entry = LauncherEntry::default();
    entry.merge(update(&[
        ("count", Value::from(4_u32)),
        ("count-visible", Value::from(true)),
    ]));
    entry.merge(update(&[
        ("progress", Value::from(1.5)),
        ("progress-visible", Value::from(true)),
    ]));
    assert_eq!(entry.badge(), Some(4));
    assert_eq!(entry.progress(), Some(1.0));
    entry.merge(update(&[("count-visible", Value::from(false))]));
    assert_eq!(entry.badge(), None);
    assert_eq!(entry.progress(), Some(1.0));
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
