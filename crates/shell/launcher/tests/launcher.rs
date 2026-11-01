#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use bande_launcher::{Action, AppInfo, Catalog, Hit, Launcher, Target};
use bande_search::Usage;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Appearance, Image, TextSystem};

struct Plain;

impl Catalog for Plain {
    fn icon(&mut self, _hit: &Hit) -> Option<Image> {
        None
    }

    fn kind(&mut self, _path: &Path, _folder: bool) -> String {
        "Document".to_owned()
    }
}

fn app(app_id: &str, name: &str, keywords: &[&str]) -> AppInfo {
    AppInfo {
        app_id: app_id.to_owned(),
        name: name.to_owned(),
        generic_name: None,
        comment: None,
        keywords: keywords.iter().map(|keyword| keyword.to_string()).collect(),
    }
}

fn press(launcher: &mut Launcher, key: Key) {
    launcher.handle(&Event::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

#[test]
fn typing_ranks_names_that_start_with_the_query_and_arrows_and_return_open_the_choice() {
    let mut launcher = Launcher::new(
        vec![
            app("org.bande.Terminal", "Terminal", &["shell"]),
            app("org.example.TextEditor", "Text Editor", &["notepad"]),
            app("org.example.Settings", "System Settings", &["preferences"]),
        ],
        Vec::new(),
        Box::new(Plain),
        PathBuf::from("/home/someone"),
        Usage::default(),
        Appearance::default(),
        TextSystem::new(),
    );
    let titles = |launcher: &Launcher| {
        launcher
            .hits()
            .into_iter()
            .map(|hit| hit.title)
            .collect::<Vec<_>>()
    };

    press(&mut launcher, Key::Character('t'));
    press(&mut launcher, Key::Character('e'));
    assert_eq!(titles(&launcher), ["Terminal", "Text Editor"]);
    press(&mut launcher, Key::Down);
    press(&mut launcher, Key::Down);
    press(&mut launcher, Key::Up);
    press(&mut launcher, Key::Down);
    press(&mut launcher, Key::Enter);
    assert_eq!(
        launcher.take_actions(),
        [Action::Open(Target::App(
            "org.example.TextEditor".to_owned()
        ))]
    );

    launcher.clear();
    for character in "prefer".chars() {
        press(&mut launcher, Key::Character(character));
    }
    assert_eq!(titles(&launcher), ["System Settings"]);
}
