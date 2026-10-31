#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_settings_panes::{Group, Kind, PaneId, SettingId, panes, search};

fn found(query: &str) -> Vec<PaneId> {
    search(&panes(), query)
        .into_iter()
        .map(|hit| hit.pane)
        .collect()
}

fn best(query: &str) -> (PaneId, Option<SettingId>) {
    let hits = search(&panes(), query);
    let hit = hits.first().expect("a hit");
    (hit.pane, hit.best().map(|setting| setting.setting))
}

#[test]
fn search_ranks_the_best_match_first_and_marks_the_matched_letters() {
    assert_eq!(found("dark"), [PaneId::Appearance]);
    assert_eq!(found("password"), [PaneId::Wifi, PaneId::Power]);
    assert_eq!(found("screen saver"), [PaneId::Power]);
    assert_eq!(found("bluetooth"), [PaneId::Bluetooth]);
    assert_eq!(found("printer"), []);
    assert_eq!(found("lock")[0], PaneId::Power);
    assert_eq!(found("rep"), [PaneId::Keyboard]);

    let hits = search(&panes(), "sleep never");
    assert_eq!(hits.len(), 1);
    let sleep = hits[0].best().expect("best setting");
    assert_eq!(sleep.setting, SettingId::SleepAfter);
    assert_eq!(sleep.label, vec![0..5]);

    let hits = search(&panes(), "display dim");
    assert_eq!(hits[0].best().expect("best").setting, SettingId::DimAfter);
    assert_eq!(hits[0].best().expect("best").label, [0..3, 8..15]);
}

#[test]
fn a_word_start_outranks_a_match_inside_a_word() {
    let hits = search(&panes(), "lock");
    let lock = hits[0].best().expect("best");
    assert_eq!(lock.setting, SettingId::LockAfter);
}

#[test]
fn the_sidebar_runs_in_four_groups_of_three_to_five_panes_and_subpages_hang_off_them() {
    let panes = panes();
    let top: Vec<PaneId> = panes
        .iter()
        .map(|pane| pane.id)
        .filter(|pane| pane.parent().is_none())
        .collect();
    for group in [
        Group::Connections,
        Group::Desktop,
        Group::Devices,
        Group::System,
    ] {
        let members = top
            .iter()
            .filter(|pane| pane.group() == Some(group))
            .count();
        assert!((3..=5).contains(&members), "{group:?} has {members} panes");
    }
    let groups: Vec<Group> = top.iter().filter_map(|pane| pane.group()).collect();
    assert_eq!(groups.len(), top.len(), "every top-level pane has a group");
    assert!(
        groups.is_sorted_by_key(|group| *group as u8),
        "panes are listed group by group"
    );
    for pane in &panes {
        if let Some(parent) = pane.id.parent() {
            assert_eq!(pane.id.group(), None);
            assert!(parent.parent().is_none() && parent.group().is_some());
            let opened = panes
                .iter()
                .find(|candidate| candidate.id == parent)
                .expect("parent")
                .settings()
                .any(|setting| setting.kind == Kind::Page(pane.id));
            assert!(opened, "{parent:?} has no row that opens {:?}", pane.id);
        }
        assert_eq!(PaneId::from_keyword(pane.id.keyword()), Some(pane.id));
    }
}

#[test]
fn every_pane_reads_from_the_english_file_and_has_search_keywords() {
    for pane in panes() {
        let keyword = pane.id.keyword();
        assert_ne!(pane.name, keyword, "{keyword} has no name");
        assert_ne!(pane.summary, format!("{keyword}-summary"));
        assert!(!pane.keywords.is_empty(), "{keyword} has no keywords");
        for word in pane
            .keywords
            .iter()
            .chain(pane.settings().flat_map(|setting| {
                assert!(
                    !setting.keywords.is_empty(),
                    "{:?} has no keywords",
                    setting.id
                );
                &setting.keywords
            }))
        {
            assert!(!word.ends_with("-keywords"), "{word} is not translated");
        }
    }
}

#[test]
fn old_pane_names_find_their_new_home() {
    assert_eq!(found("lock screen")[0], PaneId::Power);
    assert_eq!(found("battery")[0], PaneId::Power);
    assert_eq!(found("wallpaper")[0], PaneId::Appearance);
    assert_eq!(found("login items")[0], PaneId::Apps);
    assert_eq!(found("default browser")[0], PaneId::Apps);
    assert_eq!(found("clipboard")[0], PaneId::PrivacySecurity);
    assert_eq!(found("date")[0], PaneId::LanguageTime);
    assert_eq!(found("region")[0], PaneId::LanguageTime);
    assert_eq!(found("general")[0], PaneId::About);
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
}
