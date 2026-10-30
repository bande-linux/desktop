#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unbound_fs::Entry;
use unbound_launcher::{
    Action, AppInfo, Catalog, Clip, Heading, Hit, Kind, Launcher, Loaded, Mode, PaneInfo,
    SettingInfo, Target,
};
use unbound_preview::preview;
use unbound_search::{EXACT, FileHit, FileResults, PREFIX, Usage, WORD, usage_path};
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer, PointerButton};
use unbound_toolkit::{Appearance, Color, CpuRenderer, Image, Point, Scheme, TextSystem};

const SCREEN: (u32, u32) = (1024, 720);
const MIN_SSIM: f64 = 0.99;
const HOME: &str = "/home/someone";
const NOW: u64 = 1_800_000_000;

struct Plain;

impl Catalog for Plain {
    fn icon(&mut self, _hit: &Hit) -> Option<Image> {
        None
    }

    fn kind(&mut self, _path: &Path, folder: bool) -> String {
        if folder { "Folder" } else { "Document" }.to_owned()
    }
}

fn app(app_id: &str, name: &str, generic: Option<&str>, keywords: &[&str]) -> AppInfo {
    AppInfo {
        app_id: app_id.to_owned(),
        name: name.to_owned(),
        generic_name: generic.map(str::to_owned),
        comment: None,
        keywords: keywords
            .iter()
            .map(|keyword| (*keyword).to_owned())
            .collect(),
    }
}

fn apps() -> Vec<AppInfo> {
    vec![
        app(
            "org.example.Calculator",
            "Calculator",
            Some("Calculator"),
            &["math"],
        ),
        app(
            "org.unbound.Terminal",
            "Terminal",
            Some("Terminal Emulator"),
            &["shell", "prompt"],
        ),
        app(
            "org.example.TextEditor",
            "Text Editor",
            Some("Text Editor"),
            &["notepad"],
        ),
        app(
            "org.example.Settings",
            "System Settings",
            None,
            &["preferences"],
        ),
        app("org.example.Fonts", "Fonts", None, &["typeface"]),
    ]
}

fn appearance(scheme: Scheme) -> Appearance {
    Appearance {
        scheme,
        ..Appearance::default()
    }
}

fn panes() -> Vec<PaneInfo> {
    vec![
        PaneInfo {
            keyword: "appearance".to_owned(),
            name: "Appearance".to_owned(),
            keywords: vec!["theme".to_owned()],
            settings: vec![SettingInfo {
                label: "Appearance".to_owned(),
                keywords: vec!["dark".to_owned(), "light".to_owned()],
            }],
        },
        PaneInfo {
            keyword: "lock-screen".to_owned(),
            name: "Lock Screen".to_owned(),
            keywords: vec!["password".to_owned()],
            settings: vec![SettingInfo {
                label: "Turn display off after".to_owned(),
                keywords: vec!["sleep".to_owned()],
            }],
        },
    ]
}

fn launcher_with(usage: Usage, scheme: Scheme) -> Launcher {
    Launcher::new(
        apps(),
        panes(),
        Box::new(Plain),
        PathBuf::from(HOME),
        usage,
        appearance(scheme),
        TextSystem::new(),
    )
}

fn launcher() -> Launcher {
    launcher_with(Usage::default(), Scheme::Light)
}

fn titles(launcher: &Launcher) -> Vec<String> {
    launcher.hits().into_iter().map(|hit| hit.title).collect()
}

fn typed(launcher: &mut Launcher, text: &str) {
    for character in text.chars() {
        let key = if character == ' ' {
            Key::Space
        } else {
            Key::Character(character)
        };
        launcher.handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }
}

fn key(launcher: &mut Launcher, key: Key) {
    launcher.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

fn super_key(launcher: &mut Launcher, key: Key) {
    launcher.handle(&Event::Key {
        key,
        modifiers: Modifiers {
            logo: true,
            ..Modifiers::default()
        },
    });
}

fn files(launcher: &mut Launcher, hits: &[(&str, bool, u32)]) {
    let (generation, _) = launcher.file_query().expect("a new query for the index");
    launcher.set_files(FileResults {
        generation,
        hits: hits
            .iter()
            .map(|(path, folder, score)| FileHit {
                path: PathBuf::from(HOME).join(path),
                folder: *folder,
                score: *score,
            })
            .collect(),
    });
}

#[test]
fn names_that_start_with_the_query_come_before_words_and_keywords() {
    let mut launcher = launcher();
    for (query, expected) in [
        ("te", &["Terminal", "Text Editor"][..]),
        ("ed", &["Text Editor"]),
        ("set", &["System Settings"]),
        ("shell", &["Terminal"]),
        ("preferences", &["System Settings"]),
        ("ss", &["System Settings"]),
        ("trm", &["Terminal"]),
    ] {
        launcher.clear();
        typed(&mut launcher, query);
        assert_eq!(titles(&launcher), expected, "{query}");
    }
    launcher.clear();
    typed(&mut launcher, "  ");
    assert_eq!(launcher.headings(), [Heading::AllApps]);
}

#[test]
fn the_best_match_of_any_kind_is_the_top_hit_and_the_rest_are_grouped() {
    let mut launcher = launcher();
    typed(&mut launcher, "te");
    files(
        &mut launcher,
        &[
            ("Documents/tea.txt", false, PREFIX - 5),
            ("Templates", true, PREFIX - 7),
            ("Notes/the text.md", false, WORD - 5),
        ],
    );
    let hits = launcher.hits();
    let kinds: Vec<Kind> = hits.iter().map(|hit| hit.kind).collect();
    assert_eq!(titles(&launcher)[0], "Terminal");
    assert_eq!(
        kinds,
        [
            Kind::Application,
            Kind::Application,
            Kind::Folder,
            Kind::Document,
            Kind::Document
        ]
    );
    assert_eq!(hits[2].detail.as_deref(), Some("~"));
    assert_eq!(hits[3].title, "tea.txt");
    assert_eq!(hits[3].detail.as_deref(), Some("~/Documents"));
}

#[test]
fn a_file_is_the_top_hit_when_it_matches_best() {
    let mut launcher = launcher();
    typed(&mut launcher, "budget");
    files(
        &mut launcher,
        &[("Documents/budget.ods", false, PREFIX - 4)],
    );
    let selection = launcher.selection().unwrap();
    assert_eq!(selection.title, "budget.ods");
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Open(Target::Path(PathBuf::from(
            "/home/someone/Documents/budget.ods"
        )))]
    );
}

#[test]
fn an_app_named_by_a_word_of_the_query_beats_a_folder_of_exactly_that_name() {
    let mut launcher = launcher();
    typed(&mut launcher, "settings");
    files(&mut launcher, &[("src/settings", true, EXACT)]);
    assert_eq!(launcher.selection().unwrap().title, "System Settings");
}

#[test]
fn file_results_for_an_older_query_are_ignored_and_older_ones_filter_while_waiting() {
    let mut launcher = launcher();
    typed(&mut launcher, "re");
    let (old, _) = launcher.file_query().unwrap();
    typed(&mut launcher, "p");
    launcher.set_files(FileResults {
        generation: old,
        hits: vec![FileHit {
            path: PathBuf::from("/home/someone/readme.md"),
            folder: false,
            score: PREFIX,
        }],
    });
    assert!(titles(&launcher).is_empty());

    files(
        &mut launcher,
        &[
            ("report.pdf", false, PREFIX - 7),
            ("repairs.txt", false, PREFIX - 8),
        ],
    );
    typed(&mut launcher, "o");
    assert_eq!(titles(&launcher), ["report.pdf"]);
}

#[test]
fn the_selection_stays_on_the_chosen_result_when_files_arrive() {
    let mut launcher = launcher();
    typed(&mut launcher, "te");
    key(&mut launcher, Key::Down);
    assert_eq!(launcher.selection().unwrap().title, "Text Editor");
    files(&mut launcher, &[("tests", true, PREFIX - 3)]);
    assert_eq!(launcher.selection().unwrap().title, "Text Editor");
}

#[test]
fn what_was_opened_often_ranks_first() {
    let folder = tempfile::tempdir().unwrap();
    let path = usage_path(Some(folder.path()), None).unwrap();
    let now = 1_800_000_000;
    let mut usage = Usage::load(path);
    for _ in 0..3 {
        usage.record("app:org.example.TextEditor", now).unwrap();
    }
    let mut launcher = launcher_with(usage, Scheme::Light);
    launcher.set_now(now);
    typed(&mut launcher, "te");
    assert_eq!(titles(&launcher), ["Text Editor", "Terminal"]);
}

#[test]
fn opening_a_result_is_remembered() {
    let folder = tempfile::tempdir().unwrap();
    let path = usage_path(Some(folder.path()), None).unwrap();
    let mut launcher = launcher_with(Usage::load(path.clone()), Scheme::Light);
    launcher
        .record_use(&Target::App("org.example.TextEditor".to_owned()))
        .unwrap();
    assert!(Usage::load(path).boost("app:org.example.TextEditor", 0) > 0);
}

#[test]
fn settings_panes_are_found_by_name_keyword_and_setting() {
    let mut launcher = launcher();
    typed(&mut launcher, "dark");
    let hit = launcher.selection().unwrap();
    assert_eq!(hit.kind, Kind::Setting);
    assert_eq!(hit.title, "Appearance");
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Open(Target::Settings("appearance".to_owned()))]
    );

    launcher.clear();
    typed(&mut launcher, "turn display");
    let hit = launcher.selection().unwrap();
    assert_eq!(hit.title, "Lock Screen");
    assert_eq!(hit.detail.as_deref(), Some("Turn display off after"));

    launcher.clear();
    typed(&mut launcher, "lock");
    assert_eq!(launcher.selection().unwrap().title, "Lock Screen");
}

#[test]
fn a_right_click_on_a_file_offers_open_show_in_files_and_copy_path() {
    let mut launcher = launcher();
    typed(&mut launcher, "budget");
    files(&mut launcher, &[("budget.ods", false, PREFIX)]);
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    let card_x = (SCREEN.0 as f32 - unbound_launcher::CARD_WIDTH) / 2.0;
    let card_y = (SCREEN.1 as f32 * 0.22).round();
    let row = Point::new(card_x + 60.0, card_y + 61.0 + 8.0 + 26.0 + 16.0);
    launcher.handle(&Event::PointerDown(
        Pointer::at(row).with_button(PointerButton::Secondary),
    ));
    let (menu, _) = launcher.ui.take_menu_request().expect("a context menu");
    let actions: Vec<&str> = menu
        .items
        .iter()
        .filter(|item| !item.separator)
        .map(|item| item.action.as_str())
        .collect();
    assert_eq!(actions, ["open", "reveal", "copy"]);
    launcher.ui.menu_action("reveal");
    assert_eq!(
        launcher.take_actions(),
        [Action::Reveal(PathBuf::from("/home/someone/budget.ods"))]
    );
}

#[test]
fn arrows_choose_a_result_and_return_opens_it() {
    let mut launcher = launcher();
    typed(&mut launcher, "te");
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Up);
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Enter);

    assert_eq!(
        launcher.take_actions(),
        [Action::Open(Target::App(
            "org.example.TextEditor".to_owned()
        ))]
    );
}

#[test]
fn tab_and_super_arrows_move_between_groups() {
    let mut launcher = launcher();
    typed(&mut launcher, "te");
    files(
        &mut launcher,
        &[("tests", true, WORD), ("tea.txt", false, WORD)],
    );
    key(&mut launcher, Key::Tab);
    assert_eq!(launcher.selection().unwrap().title, "Text Editor");
    key(&mut launcher, Key::Tab);
    assert_eq!(launcher.selection().unwrap().title, "tests");
    super_key(&mut launcher, Key::Down);
    assert_eq!(launcher.selection().unwrap().title, "tea.txt");
    super_key(&mut launcher, Key::Up);
    assert_eq!(launcher.selection().unwrap().title, "tests");
}

#[test]
fn a_calculation_is_the_top_hit_and_return_or_super_c_copies_it() {
    let mut launcher = launcher();
    typed(&mut launcher, "1200 * 15%");
    let top = launcher.selection().unwrap();
    assert_eq!(top.kind, Kind::Calculation);
    assert_eq!(top.title, "180");
    super_key(&mut launcher, Key::Character('c'));
    assert_eq!(launcher.take_actions(), [Action::Copy("180".to_owned())]);
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Copy("180".to_owned()), Action::Close]
    );
}

#[test]
fn super_return_shows_a_file_in_files_and_super_c_copies_its_path() {
    let mut launcher = launcher();
    typed(&mut launcher, "budget");
    files(&mut launcher, &[("budget.ods", false, PREFIX)]);
    super_key(&mut launcher, Key::Enter);
    super_key(&mut launcher, Key::Character('c'));
    let path = PathBuf::from("/home/someone/budget.ods");
    assert_eq!(
        launcher.take_actions(),
        [
            Action::Reveal(path.clone()),
            Action::Copy(path.display().to_string())
        ]
    );
}

#[test]
fn escape_clears_the_query_and_then_closes() {
    let mut launcher = launcher();
    typed(&mut launcher, "te");
    key(&mut launcher, Key::Escape);
    assert_eq!(launcher.query(), "");
    assert_eq!(launcher.headings(), [Heading::AllApps]);
    assert!(launcher.take_actions().is_empty());
    key(&mut launcher, Key::Escape);
    assert_eq!(launcher.take_actions(), [Action::Close]);
}

#[test]
fn a_click_beside_the_card_closes_and_a_click_on_a_result_opens_it() {
    let mut launcher = launcher();
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    let beside = Pointer::at(Point::new(10.0, 10.0));
    launcher.handle(&Event::PointerDown(beside));
    launcher.handle(&Event::PointerUp(beside));
    assert_eq!(launcher.take_actions(), [Action::Close]);

    typed(&mut launcher, "te");
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    let card_x = (SCREEN.0 as f32 - unbound_launcher::CARD_WIDTH) / 2.0;
    let card_y = (SCREEN.1 as f32 * 0.22).round();
    let second_row = Point::new(
        card_x + 60.0,
        card_y + 61.0 + 8.0 + 26.0 + 32.0 + 26.0 + 16.0,
    );
    launcher.handle(&Event::PointerMove(Pointer::at(second_row)));
    launcher.handle(&Event::PointerDown(Pointer::at(second_row)));
    launcher.handle(&Event::PointerUp(Pointer::at(second_row)));
    assert_eq!(
        launcher.take_actions(),
        [Action::Open(Target::App(
            "org.example.TextEditor".to_owned()
        ))]
    );
}

#[test]
fn reduce_motion_fades_the_card_in_without_moving_it() {
    let mut launcher = Launcher::new(
        apps(),
        panes(),
        Box::new(Plain),
        PathBuf::from(HOME),
        Usage::default(),
        Appearance {
            reduce_motion: true,
            ..Appearance::default()
        },
        TextSystem::new(),
    );
    let shown = Instant::now();
    launcher.appear(shown);
    let scene = launcher.scene(SCREEN.0, SCREEN.1, shown + Duration::from_millis(30));
    let [unbound_toolkit::Item::Opacity { items, .. }] = scene.items.as_slice() else {
        panic!("the card fades in");
    };
    assert!(
        !items
            .iter()
            .any(|item| matches!(item, unbound_toolkit::Item::Translate { .. }))
    );
}

#[test]
fn the_card_appears_with_a_quick_fade_and_settles() {
    let mut launcher = launcher();
    let shown = Instant::now();
    launcher.appear(shown);
    assert!(launcher.is_animating(shown));
    assert!(launcher.is_animating(shown + Duration::from_millis(60)));
    assert!(!launcher.is_animating(shown + Duration::from_millis(600)));
    let scene = launcher.scene(SCREEN.0, SCREEN.1, shown + Duration::from_millis(30));
    assert!(matches!(
        scene.items.as_slice(),
        [unbound_toolkit::Item::Opacity { alpha, .. }] if *alpha > 0.0 && *alpha < 1.0
    ));
}

#[test]
fn results_are_published_as_list_options() {
    let mut launcher = launcher();
    typed(&mut launcher, "te");
    files(&mut launcher, &[("tests", true, WORD)]);
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    let accessibility = launcher.ui.accessibility();
    let options = accessibility
        .nodes
        .iter()
        .filter(|(_, node)| node.role() == accesskit::Role::ListBoxOption)
        .count();
    assert_eq!(options, 3);
}

#[test]
fn every_keystroke_updates_the_results_within_a_frame() {
    let mut apps = apps();
    for index in 0..800 {
        apps.push(app(
            &format!("org.example.App{index}"),
            &format!("Example Application {index}"),
            Some("Utility Tool"),
            &["example", "tool", "utility"],
        ));
    }
    let mut launcher = Launcher::new(
        apps,
        panes(),
        Box::new(Plain),
        PathBuf::from(HOME),
        Usage::default(),
        appearance(Scheme::Light),
        TextSystem::new(),
    );
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    let mut timings = Vec::new();
    for query in ["exam", "te", "tool 4", "settings", "2*21"] {
        launcher.clear();
        for character in query.chars() {
            let started = Instant::now();
            typed(&mut launcher, &character.to_string());
            launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
            timings.push(started.elapsed());
        }
    }
    timings.sort();
    let median = timings[timings.len() / 2];
    let worst = timings[timings.len() - 1];
    println!("keystroke to scene: median {median:?}, worst {worst:?}");
    assert!(median < Duration::from_millis(16), "median {median:?}");
}

fn check_golden(name: &str, size: (u32, u32), rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(size.0, size.1, rgba).expect("frame");
    let actual = image::DynamicImage::ImageRgba8(actual).to_rgb8();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.png"));
    if std::env::var_os("UNBOUND_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(path.parent().expect("golden directory")).expect("directory");
        actual.save(&path).expect("write golden");
    }
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    actual.save(&target).expect("save the frame");
    let expected = image::open(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
        .to_rgb8();
    let similarity = image_compare::rgb_hybrid_compare(&expected, &actual).expect("same size");
    assert!(
        similarity.score >= MIN_SSIM,
        "{name} differs from its golden: {:.4} (frame in {})",
        similarity.score,
        target.display()
    );
}

fn render(launcher: &mut Launcher, name: &str) {
    let mut scene = launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    scene.background = Color::hex(0x5A6B7D);
    for (scale, suffix) in [(1.0, "1x"), (1.5, "1.5x")] {
        let size = (
            (f64::from(SCREEN.0) * scale).round() as u32,
            (f64::from(SCREEN.1) * scale).round() as u32,
        );
        let mut rgba = vec![0; (size.0 * size.1 * 4) as usize];
        CpuRenderer::new()
            .render_into(&scene, scale, &mut rgba)
            .expect("frame");
        check_golden(&format!("{name}-{suffix}"), size, rgba);
    }
}

#[test]
fn the_launcher_matches_its_goldens() {
    let folder = tempfile::tempdir().unwrap();
    let document = folder.path().join("notes.txt");
    std::fs::write(&document, "Groceries\n- tea\n- bread\n").unwrap();
    std::fs::write(folder.path().join("Holiday plans.txt"), "Rome").unwrap();
    let xbel = recently_used_xbel(&[&folder.path().join("Holiday plans.txt")]);
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut usage = Usage::default();
        usage.record("app:org.unbound.Terminal", NOW).unwrap();
        let mut launcher = launcher_with(usage, scheme);
        launcher.set_now(NOW);
        launcher.set_recently_used(Some(&xbel));
        render(&mut launcher, &format!("launcher-browse-{suffix}"));

        typed(&mut launcher, "te");
        key(&mut launcher, Key::Down);
        render(&mut launcher, &format!("launcher-results-{suffix}"));

        launcher.clear();
        typed(&mut launcher, "5 km in mi");
        render(&mut launcher, &format!("launcher-calculation-{suffix}"));

        launcher.clear();
        typed(&mut launcher, "notes");
        let (generation, _) = launcher.file_query().unwrap();
        launcher.set_files(FileResults {
            generation,
            hits: vec![FileHit {
                path: document.clone(),
                folder: false,
                score: PREFIX,
            }],
        });
        let requested = launcher.preview_request().unwrap();
        let entry = Entry::read(requested, "notes.txt".to_owned());
        let content = preview(&entry, Image::from_rgba);
        launcher.set_preview(Loaded {
            entry: Entry {
                modified: None,
                ..entry
            },
            content,
        });
        render(&mut launcher, &format!("launcher-file-{suffix}"));

        launcher.clear();
        typed(&mut launcher, "zzzq");
        files(&mut launcher, &[]);
        render(&mut launcher, &format!("launcher-none-{suffix}"));

        launcher.open(Mode::Emoji, true);
        typed(&mut launcher, "arrow");
        key(&mut launcher, Key::Right);
        render(&mut launcher, &format!("launcher-emoji-{suffix}"));

        launcher.set_now(NOW);
        launcher.open(Mode::Clipboard, false);
        launcher.set_clips(clips());
        render(&mut launcher, &format!("launcher-clipboard-{suffix}"));
    }
}

fn recently_used_xbel(paths: &[&Path]) -> String {
    let bookmarks: String = paths
        .iter()
        .map(|path| {
            format!(
                "<bookmark href=\"file://{}\" added=\"2026-01-01T10:00:00Z\" modified=\"2026-01-01T10:00:00Z\" visited=\"2026-01-01T10:00:00Z\"></bookmark>\n",
                path.display()
            )
        })
        .collect();
    format!("<?xml version=\"1.0\"?>\n<xbel version=\"1.0\">\n{bookmarks}</xbel>\n")
}

#[test]
fn an_empty_query_shows_recent_apps_and_documents_then_every_app_alphabetically() {
    let folder = tempfile::tempdir().unwrap();
    let report = folder.path().join("report.pdf");
    std::fs::write(&report, "pdf").unwrap();
    let mut usage = Usage::default();
    usage.record("app:org.unbound.Terminal", NOW - 60).unwrap();
    usage.record("app:org.example.Fonts", NOW).unwrap();
    usage.record("app:org.example.Removed", NOW).unwrap();
    let mut launcher = launcher_with(usage, Scheme::Light);
    launcher.set_now(NOW);
    launcher.set_recently_used(Some(&recently_used_xbel(&[
        &report,
        &folder.path().join("deleted.txt"),
    ])));
    assert_eq!(
        launcher.headings(),
        [
            Heading::RecentApps,
            Heading::RecentDocuments,
            Heading::AllApps
        ]
    );
    assert_eq!(
        titles(&launcher),
        [
            "Fonts",
            "Terminal",
            "report.pdf",
            "Calculator",
            "Fonts",
            "System Settings",
            "Terminal",
            "Text Editor"
        ]
    );
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Open(Target::App("org.example.Fonts".to_owned()))]
    );
    typed(&mut launcher, "fo");
    assert_eq!(titles(&launcher), ["Fonts"]);
}

#[test]
fn arrows_move_through_the_app_grid_in_two_dimensions() {
    let mut apps = apps();
    for index in 0..20 {
        apps.push(app(
            &format!("org.example.Z{index:02}"),
            &format!("Zeta {index:02}"),
            None,
            &[],
        ));
    }
    let mut launcher = Launcher::new(
        apps,
        panes(),
        Box::new(Plain),
        PathBuf::from(HOME),
        Usage::default(),
        appearance(Scheme::Light),
        TextSystem::new(),
    );
    let selected = |launcher: &Launcher| launcher.selection().unwrap().title;
    assert_eq!(selected(&launcher), "Calculator");
    key(&mut launcher, Key::Right);
    assert_eq!(selected(&launcher), "Fonts");
    key(&mut launcher, Key::Down);
    assert_eq!(selected(&launcher), "Zeta 04");
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Down);
    assert_eq!(selected(&launcher), "Zeta 19");
    key(&mut launcher, Key::Up);
    assert_eq!(selected(&launcher), "Zeta 11");
    key(&mut launcher, Key::Left);
    assert_eq!(selected(&launcher), "Zeta 10");
}

#[test]
fn a_long_list_scrolls_to_keep_the_selection_in_view() {
    let mut apps = apps();
    for index in 0..200 {
        apps.push(app(
            &format!("org.example.App{index:03}"),
            &format!("Example {index:03}"),
            None,
            &[],
        ));
    }
    let mut launcher = Launcher::new(
        apps,
        panes(),
        Box::new(Plain),
        PathBuf::from(HOME),
        Usage::default(),
        Appearance {
            reduce_motion: true,
            ..Appearance::default()
        },
        TextSystem::new(),
    );
    let scroll = |launcher: &mut Launcher| {
        launcher.scene(SCREEN.0, SCREEN.1, Instant::now() + Duration::from_secs(5));
        launcher
            .ui
            .accessibility()
            .nodes
            .iter()
            .find(|(_, node)| node.role() == accesskit::Role::Dialog)
            .and_then(|(_, node)| node.scroll_y())
            .unwrap()
    };
    assert_eq!(scroll(&mut launcher), 0.0);
    for _ in 0..12 {
        key(&mut launcher, Key::Down);
        scroll(&mut launcher);
    }
    assert!(scroll(&mut launcher) > 500.0);
    launcher.handle(&Event::Scroll(unbound_toolkit::ui::Scroll {
        position: Point::new(SCREEN.0 as f32 / 2.0, SCREEN.1 as f32 * 0.5),
        delta: Point::new(0.0, -10_000.0),
        ..unbound_toolkit::ui::Scroll::default()
    }));
    assert_eq!(scroll(&mut launcher), 0.0);
}

#[test]
fn the_menu_key_and_shift_f10_open_the_menu_of_the_selection() {
    let mut launcher = launcher();
    typed(&mut launcher, "budget");
    files(&mut launcher, &[("budget.ods", false, PREFIX)]);
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    key(&mut launcher, Key::Menu);
    let (menu, point) = launcher.take_menu_request().expect("a context menu");
    assert_eq!(menu.items[0].action, "open");
    assert!(point.y > SCREEN.1 as f32 * 0.22 + 60.0);
    launcher.handle(&Event::Key {
        key: Key::Function(10),
        modifiers: Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    });
    assert!(launcher.take_menu_request().is_some());
    assert!(launcher.menu_action("copy"));
    assert_eq!(
        launcher.take_actions(),
        [Action::Copy("/home/someone/budget.ods".to_owned())]
    );
}

#[test]
fn result_names_mark_the_characters_that_matched() {
    let mut launcher = launcher();
    typed(&mut launcher, "ted");
    let hit = launcher.selection().unwrap();
    assert_eq!(hit.title, "Text Editor");
    assert_eq!(hit.matched, [0, 1, 6]);
    launcher.clear();
    typed(&mut launcher, "edit");
    assert_eq!(launcher.selection().unwrap().matched, [5, 6, 7, 8]);
}

fn emoji_targets(launcher: &Launcher) -> Vec<Target> {
    launcher.hits().into_iter().map(|hit| hit.target).collect()
}

#[test]
fn emoji_are_found_by_name_and_keyword_and_return_inserts_or_copies() {
    let mut launcher = launcher();
    launcher.open(Mode::Emoji, true);
    assert_eq!(launcher.headings()[0], Heading::EmojiGroup(0));
    assert!(launcher.take_actions().is_empty());
    typed(&mut launcher, "grinning face");
    assert_eq!(emoji_targets(&launcher)[0], Target::Emoji("😀".to_owned()));
    assert_eq!(launcher.selection().unwrap().title, "Grinning face");
    launcher.clear();
    typed(&mut launcher, "lol");
    assert!(emoji_targets(&launcher).contains(&Target::Emoji("😂".to_owned())));
    launcher.clear();
    typed(&mut launcher, "grinning face");
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Close, Action::Insert("😀".to_owned())]
    );

    launcher.open(Mode::Emoji, false);
    typed(&mut launcher, "grinning face");
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Copy("😀".to_owned()), Action::Close]
    );
    launcher.open(Mode::Search, false);
    typed(&mut launcher, "te");
    assert!(launcher.file_query().is_some());
    assert_eq!(titles(&launcher), ["Terminal", "Text Editor"]);
}

#[test]
fn chosen_emoji_come_first_next_time() {
    let folder = tempfile::tempdir().unwrap();
    let path = usage_path(Some(folder.path()), None).unwrap();
    let mut launcher = launcher_with(Usage::load(path), Scheme::Light);
    launcher.set_now(NOW);
    launcher.open(Mode::Emoji, false);
    typed(&mut launcher, "red heart");
    key(&mut launcher, Key::Enter);
    launcher.open(Mode::Emoji, false);
    assert_eq!(
        launcher.headings()[..2],
        [Heading::FrequentlyUsed, Heading::EmojiGroup(0)]
    );
    assert_eq!(
        emoji_targets(&launcher)[0],
        Target::Emoji("❤\u{FE0F}".to_owned())
    );
}

#[test]
fn a_skin_tone_chosen_from_the_menu_is_remembered_and_used() {
    let mut launcher = launcher();
    launcher.open(Mode::Emoji, true);
    typed(&mut launcher, "waving hand");
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    key(&mut launcher, Key::Menu);
    let (menu, _) = launcher.take_menu_request().expect("a menu");
    let tones: Vec<&str> = menu
        .items
        .iter()
        .filter(|item| item.action.starts_with("tone-"))
        .map(|item| item.title.as_str())
        .collect();
    assert_eq!(tones.len(), 6);
    assert!(tones[3].starts_with("👋\u{1F3FD}"));
    assert!(launcher.menu_action("tone-3"));
    assert_eq!(launcher.take_actions(), [Action::SkinTone(Some(2))]);
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Close, Action::Insert("👋\u{1F3FD}".to_owned())]
    );
}

fn clips() -> Vec<Clip> {
    let text = |id: u64, text: &str| Clip {
        id,
        copied: NOW - id * 60,
        text: text.to_owned(),
        mimes: vec!["text/plain;charset=utf-8".to_owned()],
    };
    vec![
        text(1, "Meeting notes\n- agenda\n- budget"),
        Clip {
            id: 2,
            copied: NOW - 120,
            text: String::new(),
            mimes: vec!["image/png".to_owned()],
        },
        Clip {
            id: 3,
            copied: NOW - 180,
            text: "file:///home/someone/Report.pdf\r\nfile:///home/someone/Budget.ods\r\n"
                .to_owned(),
            mimes: vec!["text/uri-list".to_owned()],
        },
        text(4, "https://example.org/budget"),
    ]
}

fn clipboard_launcher(inserts: bool) -> Launcher {
    let mut launcher = launcher();
    launcher.open(Mode::Clipboard, inserts);
    launcher.set_clips(clips());
    launcher
}

#[test]
fn the_clipboard_history_lists_copies_newest_first_and_filters_them() {
    let mut launcher = clipboard_launcher(false);
    assert_eq!(
        titles(&launcher),
        [
            "Meeting notes",
            "Image",
            "Report.pdf, Budget.ods",
            "https://example.org/budget"
        ]
    );
    let kinds: Vec<Kind> = launcher.hits().iter().map(|hit| hit.kind).collect();
    assert_eq!(
        kinds,
        [
            Kind::CopiedText,
            Kind::CopiedImage,
            Kind::CopiedFiles,
            Kind::CopiedText
        ]
    );
    typed(&mut launcher, "budget");
    assert_eq!(
        titles(&launcher),
        [
            "Meeting notes",
            "Report.pdf, Budget.ods",
            "https://example.org/budget"
        ]
    );
    assert!(launcher.file_query().is_none());
}

#[test]
fn return_puts_a_copy_back_on_the_clipboard_and_types_text_when_it_can() {
    let mut launcher = clipboard_launcher(false);
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::PasteClip(1), Action::Close]
    );

    let mut launcher = clipboard_launcher(true);
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [
            Action::PasteClip(1),
            Action::Close,
            Action::Insert("Meeting notes\n- agenda\n- budget".to_owned())
        ]
    );
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::PasteClip(2), Action::Close]
    );
}

#[test]
fn delete_removes_a_copy_and_clear_history_removes_them_all() {
    let mut launcher = clipboard_launcher(false);
    key(&mut launcher, Key::Down);
    key(&mut launcher, Key::Delete);
    assert_eq!(launcher.take_actions(), [Action::RemoveClip(2)]);
    assert_eq!(launcher.selection().unwrap().target, Target::Clip(3));
    assert_eq!(launcher.hits().len(), 3);
    launcher.scene(SCREEN.0, SCREEN.1, Instant::now());
    key(&mut launcher, Key::Menu);
    let (menu, _) = launcher.take_menu_request().expect("a menu");
    let actions: Vec<&str> = menu
        .items
        .iter()
        .filter(|item| !item.separator)
        .map(|item| item.action.as_str())
        .collect();
    assert_eq!(actions, ["open", "remove", "clear"]);
    assert!(launcher.menu_action("clear"));
    assert_eq!(launcher.take_actions(), [Action::ClearClips]);
    assert!(launcher.hits().is_empty());
}

#[test]
fn an_image_copy_asks_for_its_picture_once_selected() {
    let mut launcher = clipboard_launcher(false);
    assert_eq!(launcher.clip_picture_request(), None);
    key(&mut launcher, Key::Down);
    assert_eq!(
        launcher.clip_picture_request(),
        Some((2, "image/png".to_owned()))
    );
    assert_eq!(launcher.clip_picture_request(), None);
}
