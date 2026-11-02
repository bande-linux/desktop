#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use bande_config::{Appearance, BarEdge, BarLayout, Config, ConfigError, Setting};

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
fn edits_keep_comments_and_bars_written_before_the_layout_key_keep_their_layout() {
    let layout = |source: &str| {
        let bar = Config::parse(source).expect("valid").bar;
        (bar.edge, bar.layout)
    };
    assert_eq!(layout(""), (BarEdge::Left, BarLayout::Combined));
    assert_eq!(
        layout("bar {\n    edge \"bottom\"\n}\n"),
        (BarEdge::Bottom, BarLayout::Separate)
    );
    assert_eq!(
        layout("bar {\n    menu-bar \"separate\"\n}\n"),
        (BarEdge::Left, BarLayout::Separate)
    );
    let source = "// Bande settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"left\" // near the dock\n    menu-bar \"in-bar\"\n}\n";
    assert_eq!(
        Setting::BarPlacement(BarEdge::Top, BarLayout::Combined)
            .apply(source)
            .expect("edit"),
        "// Bande settings\nappearance \"dark\" // at night\n\nbar {\n    edge \"top\" // near the dock\n    layout \"combined\"\n}\n"
    );
}
