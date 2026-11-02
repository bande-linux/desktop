#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use bande_config::{Appearance, BarEdge, Config, ConfigError, Setting};

#[test]
fn kdl_problems_are_reported_where_they_are_while_every_valid_setting_applies() {
    let (config, problems) = Config::load(
        "appearance \"dark\"\ntheme \"graphite\"\nbar {\n    edge \"middle\"\n    size 52\n    sparkle #true\n}\nidle {\n    display-off-after \"10m\"\n    lock-after \"5m\"\n}\n",
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
    assert_eq!(
        config.idle.display_off_after,
        Some(Duration::from_secs(10 * 60))
    );

    let Err(ConfigError { line, column, .. }) = Config::load("bar {\n    edge \"left\n}\n") else {
        panic!("an unterminated string is a syntax error");
    };
    assert_eq!((line, column), (2, 10));
}

#[test]
fn edits_keep_comments() {
    let source = "// Bande settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"left\" // near the dock\n}\n";
    assert_eq!(
        Config::parse(source).expect("valid").bar.edge,
        BarEdge::Left
    );
    assert_eq!(
        Setting::BarEdge(BarEdge::Top).apply(source).expect("edit"),
        "// Bande settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"top\" // near the dock\n}\n"
    );
}
