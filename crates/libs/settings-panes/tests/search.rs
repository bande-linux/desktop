#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_settings_panes::{PaneId, SettingId, panes, search};

#[test]
fn search_ranks_the_best_match_first_and_marks_the_matched_letters() {
    let panes = panes();
    let found = |query: &str| {
        search(&panes, query)
            .into_iter()
            .map(|hit| hit.pane)
            .collect::<Vec<_>>()
    };

    assert_eq!(found("dark"), [PaneId::Appearance]);
    assert_eq!(found("password"), [PaneId::Wifi, PaneId::LockScreen]);
    assert_eq!(found("screen saver"), [PaneId::LockScreen]);
    assert_eq!(found("bluetooth"), [PaneId::Bluetooth]);
    assert_eq!(found("printer"), []);
    assert_eq!(found("lock")[0], PaneId::LockScreen);
    assert_eq!(found("rep"), [PaneId::Keyboard]);

    let hits = search(&panes, "sleep never");
    assert_eq!(hits.len(), 1);
    let sleep = hits[0].best().expect("best setting");
    assert_eq!(sleep.setting, SettingId::SleepAfter);
    assert_eq!(sleep.label, vec![0..5]);

    let hits = search(&panes, "display dim");
    assert_eq!(hits[0].best().expect("best").setting, SettingId::DimAfter);
    assert_eq!(hits[0].best().expect("best").label, [0..3, 8..15]);
}

#[test]
fn a_word_start_outranks_a_match_inside_a_word() {
    let panes = panes();
    let hits = search(&panes, "lock");
    let lock = hits[0].best().expect("best");
    assert_eq!(lock.setting, SettingId::LockAfter);
}

#[test]
fn every_setting_has_search_keywords_and_general_pages_belong_to_general() {
    for pane in panes() {
        assert!(!pane.keywords.is_empty(), "{:?} has no keywords", pane.id);
        for setting in pane.settings() {
            assert!(
                !setting.keywords.is_empty(),
                "{:?} has no keywords",
                setting.id
            );
        }
    }
    let panes = panes();
    let best = |query: &str| {
        let hits = search(&panes, query);
        let hit = hits.first().expect("a hit");
        (hit.pane, hit.best().map(|setting| setting.setting))
    };
    assert_eq!(
        best("hostname"),
        (PaneId::About, Some(SettingId::ComputerName))
    );
    assert_eq!(
        best("serial number"),
        (PaneId::About, Some(SettingId::AboutThisComputer))
    );
    assert_eq!(
        best("empty trash"),
        (PaneId::Storage, Some(SettingId::StorageUsage))
    );
    assert_eq!(
        best("free up space"),
        (PaneId::Storage, Some(SettingId::LargeFiles))
    );
    assert_eq!(PaneId::About.parent(), Some(PaneId::General));
    assert_eq!(PaneId::General.parent(), None);
}
