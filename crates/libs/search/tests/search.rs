#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use unbound_search::{
    EXACT, FILE_KEY, FileIndex, FileResults, PREFIX, Query, Usage, calculate, fold, recent_files,
    usage_path,
};

fn score(query: &str, candidate: &str) -> Option<u32> {
    Query::new(query).unwrap().score(&fold(candidate))
}

#[test]
fn matches_rank_exact_then_prefix_then_word_then_initials_then_substring_then_fuzzy() {
    let ranked = [
        score("files", "Files"),
        score("fil", "Files"),
        score("fil", "Open Files"),
        score("tax 2024", "2024 Tax Return.pdf"),
        score("vsc", "Visual Studio Code"),
        score("ile", "Files"),
        score("fls", "Files"),
    ];
    assert!(ranked.iter().all(Option::is_some), "{ranked:?}");
    assert!(
        ranked.windows(2).all(|pair| pair[0] > pair[1]),
        "{ranked:?}"
    );
    assert_eq!(ranked[0], Some(EXACT));
    assert!(score("zq", "Files").is_none());
    assert!(score("sfl", "Files").is_none());
    assert!(Query::new("   ").is_none());
}

#[test]
fn shorter_names_win_among_prefix_matches() {
    assert!(score("term", "Terminal") > score("term", "Terminal Emulator Settings"));
    assert!(score("term", "Terminal").unwrap() < PREFIX);
}

#[test]
fn widely_scattered_letters_do_not_match() {
    assert!(score("aez", "abcdefghijklmnopqrstuvwxyz").is_none());
}

#[test]
fn the_calculator_evaluates_arithmetic_with_precedence() {
    assert_eq!(calculate("2+3*4").as_deref(), Some("14"));
    assert_eq!(calculate("(2+3)*4").as_deref(), Some("20"));
    assert_eq!(calculate("2^3^2").as_deref(), Some("512"));
    assert_eq!(calculate("-2^2").as_deref(), Some("-4"));
    assert_eq!(calculate("10 / 4").as_deref(), Some("2.5"));
    assert_eq!(calculate("1/3").as_deref(), Some("0.333333333333"));
    assert_eq!(calculate("0.1 + 0.2").as_deref(), Some("0.3"));
    assert_eq!(calculate("3 × 4 ÷ 2").as_deref(), Some("6"));
    assert_eq!(calculate("1,299.50 * 2").as_deref(), Some("2599"));
    assert_eq!(calculate("sqrt 16 + 1").as_deref(), Some("5"));
    assert_eq!(calculate("2 * pi").as_deref(), Some("6.28318530718"));
    assert_eq!(calculate("2^64").as_deref(), Some("1.84467440737e19"));
}

#[test]
fn the_calculator_applies_percentages_as_people_mean_them() {
    assert_eq!(calculate("200 + 10%").as_deref(), Some("220"));
    assert_eq!(calculate("80 - 25%").as_deref(), Some("60"));
    assert_eq!(calculate("15% of 200").as_deref(), Some("30"));
    assert_eq!(calculate("200 * 15%").as_deref(), Some("30"));
}

#[test]
fn the_calculator_converts_units() {
    assert_eq!(calculate("5 km in mi").as_deref(), Some("3.10685596119 mi"));
    assert_eq!(calculate("100 f to c").as_deref(), Some("37.7777777778°C"));
    assert_eq!(calculate("-40 °C in fahrenheit").as_deref(), Some("-40°F"));
    assert_eq!(calculate("5 in in cm").as_deref(), Some("12.7 cm"));
    assert_eq!(calculate("6 feet to m").as_deref(), Some("1.8288 m"));
    assert_eq!(calculate("1 GiB in MB").as_deref(), Some("1073.741824 MB"));
    assert_eq!(
        calculate("100 km/h in mph").as_deref(),
        Some("62.1371192237 mph")
    );
    assert_eq!(calculate("90 minutes in hours").as_deref(), Some("1.5 h"));
    assert_eq!(calculate("1 km + 300 m").as_deref(), Some("1.3 km"));
    assert_eq!(calculate("10 km / 2").as_deref(), Some("5 km"));
    assert_eq!(calculate("1 km / 250 m").as_deref(), Some("4"));
}

#[test]
fn the_calculator_ignores_what_is_not_a_calculation() {
    for text in [
        "42",
        "5 km",
        "terminal",
        "2024 tax",
        "2 +",
        "5 km + 2 kg",
        "1/0",
        "5 kg in km",
        "km",
    ] {
        assert_eq!(calculate(text), None, "{text}");
    }
}

#[test]
fn usage_boosts_what_was_opened_often_and_lately_and_persists() {
    let folder = tempfile::tempdir().unwrap();
    let file = usage_path(Some(folder.path()), None).unwrap();
    let now = 1_000_000_000;
    let mut usage = Usage::load(file.clone());
    for _ in 0..5 {
        usage.record("app:org.example.Often", now).unwrap();
    }
    usage.record("app:org.example.Once", now).unwrap();
    usage
        .record("app:org.example.Long.Ago", now - 200 * 86_400)
        .unwrap();

    let reloaded = Usage::load(file);
    let often = reloaded.boost("app:org.example.Often", now);
    let once = reloaded.boost("app:org.example.Once", now);
    let long_ago = reloaded.boost("app:org.example.Long.Ago", now);
    assert!(often > once && once > long_ago && long_ago > 0);
    assert_eq!(reloaded.boost("app:org.example.Never", now), 0);
    assert!(often <= unbound_search::MAX_BOOST);
}

#[test]
fn usage_lives_in_the_state_folder() {
    assert_eq!(
        usage_path(None, Some(Path::new("/home/someone"))).unwrap(),
        Path::new("/home/someone/.local/state/unbound/launcher-usage")
    );
    assert_eq!(
        usage_path(Some(Path::new("relative")), Some(Path::new("/home/a"))).unwrap(),
        Path::new("/home/a/.local/state/unbound/launcher-usage")
    );
}

fn wait_for(
    index: &mut FileIndex,
    generation: u64,
    text: &str,
    wanted: impl Fn(&FileResults) -> bool,
) -> FileResults {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        index.query(generation, text);
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(200) {
            if let Some(results) = index.take()
                && results.generation == generation
                && wanted(&results)
            {
                return results;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(Instant::now() < deadline, "no results for {text}");
    }
}

fn names(results: &FileResults) -> Vec<String> {
    results
        .hits
        .iter()
        .map(|hit| hit.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn the_index_finds_files_and_folders_but_not_hidden_or_cached_ones() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    fs::create_dir_all(root.join("Documents/Taxes")).unwrap();
    fs::write(root.join("Documents/Taxes/report 2024.pdf"), "").unwrap();
    fs::write(root.join("Documents/report-draft.txt"), "").unwrap();
    fs::create_dir_all(root.join(".config/report")).unwrap();
    fs::write(root.join(".report-hidden"), "").unwrap();
    fs::create_dir_all(root.join("code/target/debug")).unwrap();
    fs::write(root.join("code/target/CACHEDIR.TAG"), "").unwrap();
    fs::write(root.join("code/target/debug/report.o"), "").unwrap();
    fs::create_dir_all(root.join("code/node_modules/report")).unwrap();

    let mut index = FileIndex::spawn(root.to_path_buf()).unwrap();
    let results = wait_for(&mut index, 1, "report", |results| results.hits.len() >= 2);
    assert_eq!(names(&results), ["report 2024.pdf", "report-draft.txt"]);

    let results = wait_for(&mut index, 2, "tax", |results| !results.hits.is_empty());
    assert_eq!(names(&results), ["Taxes"]);
    assert!(results.hits[0].folder);

    let taxes = root.join("Documents/Taxes");
    let deadline = Instant::now() + Duration::from_secs(10);
    let within = loop {
        index.query_within(3, "report", &taxes, 500);
        std::thread::sleep(Duration::from_millis(20));
        if let Some(results) = index.take().filter(|results| results.generation == 3) {
            break results;
        }
        assert!(Instant::now() < deadline, "no results within the folder");
    };
    assert_eq!(names(&within), ["report 2024.pdf"]);
}

#[test]
fn the_index_follows_changes_in_the_home_folder() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    fs::create_dir_all(root.join("Pictures")).unwrap();
    fs::write(root.join("Pictures/holiday.png"), "").unwrap();
    let mut index = FileIndex::spawn(root.to_path_buf()).unwrap();
    wait_for(&mut index, 1, "holiday", |results| results.hits.len() == 1);

    fs::create_dir_all(root.join("Pictures/Trip")).unwrap();
    fs::write(root.join("Pictures/Trip/holiday beach.png"), "").unwrap();
    fs::rename(
        root.join("Pictures/holiday.png"),
        root.join("Pictures/summer.png"),
    )
    .unwrap();
    let results = wait_for(&mut index, 2, "holiday", |results| {
        names(results) == ["holiday beach.png"]
    });
    assert_eq!(results.hits.len(), 1);
    wait_for(&mut index, 3, "summer", |results| {
        names(results) == ["summer.png"]
    });

    fs::remove_dir_all(root.join("Pictures/Trip")).unwrap();
    wait_for(&mut index, 4, "holiday", |results| results.hits.is_empty());
}

#[test]
fn opened_files_are_boosted() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    fs::write(root.join("notes.txt"), "").unwrap();
    fs::write(root.join("notes-old.txt"), "").unwrap();
    let mut index = FileIndex::spawn(root.to_path_buf()).unwrap();
    let results = wait_for(&mut index, 1, "notes", |results| results.hits.len() == 2);
    assert_eq!(names(&results), ["notes.txt", "notes-old.txt"]);
    index.set_boosts([(root.join("notes-old.txt"), 150)].into_iter().collect());
    let results = wait_for(&mut index, 2, "notes", |results| results.hits.len() == 2);
    assert_eq!(names(&results), ["notes-old.txt", "notes.txt"]);
}

#[test]
fn searching_a_large_index_is_fast() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    for folder in 0..100 {
        let path = root.join(format!("project {folder}"));
        fs::create_dir(&path).unwrap();
        for file in 0..200 {
            fs::write(path.join(format!("document {file} draft {folder}.txt")), "").unwrap();
        }
    }
    let mut index = FileIndex::spawn(root.to_path_buf()).unwrap();
    wait_for(&mut index, 1, "project 99", |results| {
        results.hits.len() == 1
    });
    let started = Instant::now();
    let results = wait_for(&mut index, 2, "dr 17", |results| !results.hits.is_empty());
    let elapsed = started.elapsed();
    assert_eq!(results.hits.len(), unbound_search::MAX_HITS);
    assert!(
        elapsed < Duration::from_millis(150),
        "{elapsed:?} for 20,100 entries"
    );
}

#[test]
fn recents_merge_the_shared_list_with_launcher_use_newest_first() {
    let home = tempfile::tempdir().unwrap();
    let root = home.path();
    for name in ["report.pdf", "photo & sea.png", "notes.txt"] {
        fs::write(root.join(name), "").unwrap();
    }
    fs::create_dir(root.join("Folder")).unwrap();
    let uri = |name: &str| unbound_xdg::file_uri(&root.join(name)).replace('&', "&amp;");
    let bookmark = |name: &str, day: u32| {
        format!(
            "<bookmark href=\"{}\" added=\"2026-01-0{day}T10:00:00Z\" modified=\"2026-01-0{day}T10:00:00.5Z\" visited=\"2026-01-01T10:00:00Z\">\n</bookmark>\n",
            uri(name)
        )
    };
    let xbel = format!(
        "<xbel>\n{}{}{}{}</xbel>\n",
        bookmark("report.pdf", 2),
        bookmark("photo & sea.png", 3),
        bookmark("gone.txt", 9),
        bookmark("Folder", 8),
    );
    let mut usage = Usage::default();
    let notes = root.join("notes.txt");
    usage
        .record(&format!("{FILE_KEY}{}", notes.display()), 1_767_900_000)
        .unwrap();
    usage
        .record(
            &format!("{FILE_KEY}{}", root.join("report.pdf").display()),
            1_000,
        )
        .unwrap();
    usage
        .record("app:org.unbound.Files", 1_800_000_000)
        .unwrap();
    assert_eq!(
        recent_files(Some(&xbel), &usage, 10),
        [notes, root.join("photo & sea.png"), root.join("report.pdf")]
    );
    assert_eq!(recent_files(Some(&xbel), &usage, 1).len(), 1);
}
