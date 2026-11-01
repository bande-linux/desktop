use bande_app_menu::{
    Entry, Standard, app_submenu_quitting_on, close_window, settings, window_submenu,
};
use bande_toolkit::ui::{Event, shortcut_pressed};

use crate::{Info, Keys, Menus};

impl Keys {
    fn close(self) -> &'static str {
        match self {
            Self::Standard => "Ctrl+W",
            Self::Shifted => "Shift+Ctrl+W",
        }
    }

    fn quit(self) -> &'static str {
        match self {
            Self::Standard => "Ctrl+Q",
            Self::Shifted => "Shift+Ctrl+Q",
        }
    }

    fn settings(self) -> &'static str {
        match self {
            Self::Standard => "Ctrl+,",
            Self::Shifted => "Shift+Ctrl+,",
        }
    }

    pub(crate) fn standard(self, event: &Event, settings: bool) -> Option<Standard> {
        [
            (self.close(), Standard::CloseWindow),
            (self.quit(), Standard::Quit),
            (self.settings(), Standard::Settings),
        ]
        .into_iter()
        .filter(|(_, standard)| settings || *standard != Standard::Settings)
        .find(|(shortcut, _)| shortcut_pressed(event, shortcut))
        .map(|(_, standard)| standard)
    }
}

pub(crate) fn assemble(info: &Info, menus: Menus) -> Vec<Entry> {
    let name = (info.name)();
    let items = match info.settings {
        Some(_) => vec![settings().shortcut(info.keys.settings())],
        None => Vec::new(),
    };
    let mut entries = vec![app_submenu_quitting_on(
        &name,
        info.version,
        items,
        info.keys.quit(),
    )];
    entries.extend(menus.menus);
    let mut window = vec![close_window().shortcut(info.keys.close())];
    window.extend(menus.window);
    entries.push(window_submenu(window));
    entries
}

#[cfg(test)]
mod tests {
    use bande_app_menu::{
        Entry, Standard, app_submenu, app_submenu_quitting_on, close_window, settings,
        window_submenu,
    };
    use bande_toolkit::Appearance;
    use bande_toolkit::ui::{Event, Key, Modifiers};

    use super::assemble;
    use crate::{Info, Keys, Menus, Settings, View};

    const INFO: Info = Info {
        id: "org.bande.Example",
        package: "bande-example",
        version: "1.2.3",
        name: || "Example".to_owned(),
        keys: Keys::Standard,
        settings: None,
    };

    fn no_view(_appearance: Appearance) -> Box<dyn View> {
        unreachable!()
    }

    fn with_settings(keys: Keys) -> Info {
        Info {
            keys,
            settings: Some(Settings {
                title: || "Example Settings".to_owned(),
                size: (400, 300),
                view: no_view,
            }),
            ..INFO
        }
    }

    fn key(character: char, shift: bool) -> Event {
        Event::Key {
            key: Key::Character(match shift {
                true => character.to_ascii_uppercase(),
                false => character,
            }),
            modifiers: Modifiers {
                control: true,
                shift,
                ..Modifiers::default()
            },
        }
    }

    #[test]
    fn the_app_menu_comes_first_and_the_window_menu_last_around_the_apps_own() {
        let file = Entry::submenu("File", vec![Entry::item("New", "new").shortcut("Ctrl+N")]);
        let tab = Entry::item("Next Tab", "next-tab");
        let menus = Menus {
            menus: vec![file.clone()],
            window: vec![tab.clone()],
        };
        assert_eq!(
            assemble(&INFO, menus),
            [
                app_submenu("Example", "1.2.3", Vec::new()),
                file,
                window_submenu(vec![close_window(), tab]),
            ]
        );
    }

    #[test]
    fn shifted_keys_move_close_and_quit_to_shift_ctrl_in_the_menus() {
        assert_eq!(
            assemble(&with_settings(Keys::Shifted), Menus::default()),
            [
                app_submenu_quitting_on(
                    "Example",
                    "1.2.3",
                    vec![settings().shortcut("Shift+Ctrl+,")],
                    "Shift+Ctrl+Q"
                ),
                window_submenu(vec![close_window().shortcut("Shift+Ctrl+W")]),
            ]
        );
    }

    #[test]
    fn settings_sit_in_the_app_menu_only_for_an_app_with_settings() {
        assert_eq!(
            assemble(&with_settings(Keys::Standard), Menus::default())[0],
            app_submenu("Example", "1.2.3", vec![settings()])
        );
    }

    #[test]
    fn close_quit_and_settings_follow_the_apps_keys() {
        let standard =
            |character, shift, settings| Keys::Standard.standard(&key(character, shift), settings);
        let shifted =
            |character, shift, settings| Keys::Shifted.standard(&key(character, shift), settings);
        assert_eq!(standard('w', false, false), Some(Standard::CloseWindow));
        assert_eq!(standard('q', false, false), Some(Standard::Quit));
        assert_eq!(standard(',', false, true), Some(Standard::Settings));
        assert_eq!(standard(',', false, false), None);
        assert_eq!(standard('w', true, false), None);
        assert_eq!(shifted('w', false, true), None);
        assert_eq!(shifted('q', false, true), None);
        assert_eq!(shifted(',', false, true), None);
        assert_eq!(shifted('w', true, false), Some(Standard::CloseWindow));
        assert_eq!(shifted('q', true, false), Some(Standard::Quit));
        assert_eq!(shifted(',', true, true), Some(Standard::Settings));
    }
}
