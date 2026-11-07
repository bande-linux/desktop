#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use bande_settings::{assignments, parse_assignments, supported_locales};

fn current(lines: &[&str]) -> Vec<String> {
    lines.iter().map(|line| (*line).to_owned()).collect()
}

const SWEDISH_FORMATS: [&str; 5] = [
    "LC_TIME=sv_SE.UTF-8",
    "LC_NUMERIC=sv_SE.UTF-8",
    "LC_MONETARY=sv_SE.UTF-8",
    "LC_MEASUREMENT=sv_SE.UTF-8",
    "LC_PAPER=sv_SE.UTF-8",
];

#[test]
fn every_offered_locale_is_named_as_localed_finds_it_in_supported() {
    let supported =
        "aa_ER UTF-8\nen_SE.UTF-8 UTF-8\nbe_BY@latin UTF-8\nen_GB ISO-8859-1\neo UTF-8\n";
    let mut codes: Vec<String> = supported_locales(supported, Path::new("/nonexistent"))
        .into_iter()
        .map(|locale| locale.code)
        .collect();
    codes.sort();
    assert_eq!(codes, ["aa_ER", "be_BY@latin", "en_SE.UTF-8"]);
}

#[test]
fn localed_refuses_a_language_list_so_a_change_of_language_never_sends_one() {
    let wanted = assignments(&current(&["LANG=C.UTF-8"]), "en_SE.UTF-8", None);
    assert_eq!(wanted, ["LANG=en_SE.UTF-8"]);

    let wanted = assignments(
        &current(&["LANG=en_AU.UTF-8", "LANGUAGE=en_AU:en_GB"]),
        "sv_SE.UTF-8",
        None,
    );
    assert_eq!(wanted, ["LANG=sv_SE.UTF-8", "LANGUAGE=sv_SE"]);

    let wanted = assignments(
        &current(&["LANG=en_AU.UTF-8", "LANGUAGE=en_AU:en_GB"]),
        "en_AU.UTF-8",
        Some("sv_SE.UTF-8"),
    );
    assert_eq!(wanted[0], "LANG=en_AU.UTF-8");
    assert_eq!(wanted[1..], SWEDISH_FORMATS);
}

#[test]
fn localed_keeps_what_it_is_not_sent_so_same_as_language_overwrites_the_formats() {
    let mut set = current(&["LANG=en_SE.UTF-8"]);
    set.extend(SWEDISH_FORMATS.map(str::to_owned));
    assert_eq!(
        parse_assignments(&set),
        ("en_SE.UTF-8".to_owned(), Some("sv_SE.UTF-8".to_owned()))
    );

    let wanted = assignments(&set, "en_SE.UTF-8", None);
    assert_eq!(wanted.len(), 6);
    assert!(wanted.contains(&"LC_TIME=en_SE.UTF-8".to_owned()));
    assert_eq!(parse_assignments(&wanted), ("en_SE.UTF-8".to_owned(), None));
}

#[test]
fn a_system_without_a_language_still_sends_a_valid_one() {
    let (language, region) = parse_assignments(&[]);
    assert_eq!((language.as_str(), region), ("C.UTF-8", None));
}
