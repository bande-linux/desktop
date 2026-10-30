#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use unbound_fs::Entry;
use unbound_launcher::{
    Action, AppInfo, Catalog, Hit, Kind, Launcher, Loaded, PaneInfo, SettingInfo, Target,
};
use unbound_preview::preview;
use unbound_search::{FileHit, FileResults, PREFIX, Usage, WORD, usage_path};
use unbound_toolkit::ui::{Event, Key, Modifiers, Pointer, PointerButton};
use unbound_toolkit::{Appearance, Color, CpuRenderer, Image, Point, Scheme, TextSystem};

const SCREEN: (u32, u32) = (1024, 720);
const MIN_SSIM: f64 = 0.99;
const HOME: &str = "/home/someone";

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
    assert!(launcher.hits().is_empty());
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
    assert!(launcher.hits().is_empty());
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

fn check_golden(name: &str, rgba: Vec<u8>) {
    let actual = image::RgbaImage::from_raw(SCREEN.0, SCREEN.1, rgba).expect("frame");
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
    let frame = CpuRenderer::new().render(&scene).expect("frame");
    check_golden(name, frame.rgba);
}

#[test]
fn the_launcher_matches_its_goldens() {
    let folder = tempfile::tempdir().unwrap();
    let document = folder.path().join("notes.txt");
    std::fs::write(&document, "Groceries\n- tea\n- bread\n").unwrap();
    for (scheme, suffix) in [(Scheme::Light, "light"), (Scheme::Dark, "dark")] {
        let mut launcher = launcher_with(Usage::default(), scheme);
        render(&mut launcher, &format!("launcher-empty-{suffix}"));

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
    }
}
