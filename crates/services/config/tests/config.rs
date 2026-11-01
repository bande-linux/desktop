#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use bande_config::{Appearance, BarEdge, Config, ConfigError, SearchCategory, Setting};

#[test]
fn kdl_problems_are_reported_where_they_are_while_every_valid_setting_applies() {
    let (config, problems) = Config::load(
        "appearance \"dark\"\ntheme \"graphite\"\nbar {\n    edge \"middle\"\n    size 52\n    sparkle #true\n}\nidle {\n    dim-after \"10m\"\n    lock-after \"5m\"\n}\n",
    )
    .expect("well-formed KDL");
    assert_eq!(
        problems
            .iter()
            .map(|problem| (problem.line, problem.column))
            .collect::<Vec<_>>(),
        [(2, 1), (4, 10), (6, 5), (10, 5)]
    );
    assert_eq!(config.appearance, Appearance::Dark);
    assert_eq!(config.bar.edge, BarEdge::Left);
    assert_eq!(config.bar.icon_size, 52);
    assert_eq!(config.idle.lock_after, Duration::from_secs(10 * 60));

    let Err(ConfigError { line, column, .. }) = Config::load("bar {\n    edge \"left\n}\n") else {
        panic!("an unterminated string is a syntax error");
    };
    assert_eq!((line, column), (2, 10));
}

#[test]
fn changing_a_setting_keeps_every_comment_and_line_around_it() {
    let source = "// Bande settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"left\" // near the dock\n}\n";
    assert_eq!(
        Setting::BarEdge(BarEdge::Top).apply(source).expect("edit"),
        "// Bande settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"top\" // near the dock\n}\n"
    );
    assert_eq!(
        Setting::Appearance(Appearance::Light)
            .apply("")
            .expect("edit"),
        "appearance \"light\"\n"
    );
}

#[test]
fn search_hides_categories_and_excludes_folders_and_writes_them_back() {
    let (config, problems) = Config::load(
        "search {\n    folders #false\n    exclude \"~/Private\" \"/srv/tax\" \"notes\"\n}\n",
    )
    .expect("well-formed KDL");
    assert_eq!(problems.len(), 1);
    assert!(!config.search.shows(SearchCategory::Folders));
    assert!(config.search.shows(SearchCategory::RecentDocuments));
    assert!(config.search.exclude.is_empty());

    let mut search = Config::parse("search {\n    folders #false\n}\n")
        .expect("valid")
        .search;
    search.set_shown(SearchCategory::Folders, true);
    search.set_shown(SearchCategory::RecentDocuments, false);
    search.exclude = vec![PathBuf::from("~/Private"), PathBuf::from("/srv/tax")];
    let edited = Setting::Search(search.clone())
        .apply("search {\n    folders #false\n}\n")
        .expect("edit");
    assert_eq!(
        edited,
        "search {\n    recent-documents #false\n    exclude \"~/Private\" \"/srv/tax\"\n}\n"
    );
    assert_eq!(Config::parse(&edited).expect("valid").search, search);
    assert_eq!(
        search.excluded(Some(Path::new("/home/ada"))),
        [
            PathBuf::from("/home/ada/Private"),
            PathBuf::from("/srv/tax")
        ]
    );
}
