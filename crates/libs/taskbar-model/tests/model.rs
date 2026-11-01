#![allow(clippy::expect_used)]

use bande_taskbar_model::bus::{app_id_from_uri, launcher_update};
use bande_taskbar_model::{LauncherEntry, arrange};
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
    entry.merge(update(&[("progress", Value::from(f64::NAN))]));
    assert_eq!(entry.progress(), Some(1.0));
}
