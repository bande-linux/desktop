#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use xdg_desktop_portal_unbound::dialog_process::{DialogProcess, arguments, decode, encode, parse};
use xdg_desktop_portal_unbound::file_chooser::{
    Choice, Dialog, FileChooserUi, Filter, Mode, Outcome, Pattern, Selection,
};
use xdg_desktop_portal_unbound::request::Cancellation;

fn dialog(mode: Mode) -> Dialog {
    let images = Filter {
        name: "Images".to_owned(),
        patterns: vec![
            Pattern::Glob("*.png".to_owned()),
            Pattern::MimeType("image/jpeg".to_owned()),
        ],
    };
    Dialog {
        app_id: "org.example.Viewer".to_owned(),
        parent_window: String::new(),
        title: "Open Images".to_owned(),
        accept_label: Some("_Import".to_owned()),
        modal: true,
        mode,
        filters: vec![
            images.clone(),
            Filter {
                name: "All files".to_owned(),
                patterns: vec![Pattern::Glob("*".to_owned())],
            },
        ],
        current_filter: Some(images),
        choices: Vec::new(),
        folder: Some(PathBuf::from("/home/anna/Pictures")),
    }
}

#[test]
fn the_dialog_survives_the_trip_through_the_command_line() {
    for mode in [
        Mode::Open {
            multiple: true,
            directory: false,
        },
        Mode::Open {
            multiple: false,
            directory: true,
        },
        Mode::Save {
            name: Some("Untitled.png".to_owned()),
            file: Some(PathBuf::from("/home/anna/old.png")),
        },
        Mode::SaveMany { names: Vec::new() },
    ] {
        let dialog = dialog(mode);
        assert_eq!(parse(arguments(&dialog)), Ok(dialog));
    }
}

#[test]
fn a_current_filter_the_app_did_not_list_is_offered_too() {
    let mut dialog = dialog(Mode::Open {
        multiple: false,
        directory: false,
    });
    let text = Filter {
        name: "Text".to_owned(),
        patterns: vec![Pattern::MimeType("text/plain".to_owned())],
    };
    dialog.current_filter = Some(text.clone());
    let parsed = parse(arguments(&dialog)).expect("parsed");
    assert_eq!(parsed.filters.last(), Some(&text));
    assert_eq!(parsed.current_filter, Some(text));
}

#[test]
fn the_answer_carries_paths_the_filter_and_the_initial_choices() {
    let mut dialog = dialog(Mode::Open {
        multiple: true,
        directory: false,
    });
    dialog.choices = vec![Choice {
        id: "encoding".to_owned(),
        label: "Encoding".to_owned(),
        options: Vec::new(),
        initial: "utf8".to_owned(),
    }];
    let selection = Selection {
        paths: vec![
            PathBuf::from("/home/anna/a b.png"),
            PathBuf::from("/home/anna/c.png"),
        ],
        choices: vec![("encoding".to_owned(), "utf8".to_owned())],
        filter: Some(dialog.filters[1].clone()),
        writable: true,
    };
    assert_eq!(decode(&encode(&selection, &dialog), &dialog), selection);
}

fn script(directory: &Path, body: &str) -> PathBuf {
    let path = directory.join("dialog");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn outcome(program: PathBuf) -> Outcome {
    let dialog = dialog(Mode::Open {
        multiple: false,
        directory: false,
    });
    DialogProcess { program }.choose(&dialog, &Cancellation::default())
}

#[test]
fn the_dialog_process_answers_chosen_cancelled_or_unavailable() {
    let directory = tempfile::tempdir().expect("temp dir");
    let chosen = outcome(script(directory.path(), "printf '0\\000/srv/a.png\\000'"));
    let Outcome::Chosen(selection) = chosen else {
        panic!("{chosen:?}");
    };
    assert_eq!(selection.paths, [PathBuf::from("/srv/a.png")]);
    assert_eq!(
        selection.filter.map(|filter| filter.name),
        Some("Images".to_owned())
    );

    assert_eq!(
        outcome(script(directory.path(), "exit 1")),
        Outcome::Cancelled
    );
    assert_eq!(
        outcome(script(directory.path(), "exit 2")),
        Outcome::Unavailable
    );
    assert_eq!(
        outcome(directory.path().join("missing")),
        Outcome::Unavailable
    );
}
