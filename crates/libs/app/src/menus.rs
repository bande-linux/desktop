use bande_app_menu::{Entry, Standard, app_submenu_quitting_on, close_window, window_submenu};
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

    pub(crate) fn standard(self, event: &Event) -> Option<Standard> {
        [
            (self.close(), Standard::CloseWindow),
            (self.quit(), Standard::Quit),
        ]
        .into_iter()
        .find(|(shortcut, _)| shortcut_pressed(event, shortcut))
        .map(|(_, standard)| standard)
    }
}

pub(crate) fn assemble(info: &Info, menus: Menus) -> Vec<Entry> {
    let name = (info.name)();
    let mut entries = vec![app_submenu_quitting_on(
        &name,
        info.version,
        Vec::new(),
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
        Entry, Standard, app_submenu, app_submenu_quitting_on, close_window, window_submenu,
    };
    use bande_toolkit::ui::{Event, Key, Modifiers};

    use super::assemble;
    use crate::{Info, Keys, Menus};

    const INFO: Info = Info {
        id: "org.bande.Example",
        package: "bande-example",
        version: "1.2.3",
        name: || "Example".to_owned(),
        keys: Keys::Standard,
    };

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
        let info = Info {
            keys: Keys::Shifted,
            ..INFO
        };
        assert_eq!(
            assemble(&info, Menus::default()),
            [
                app_submenu_quitting_on("Example", "1.2.3", Vec::new(), "Shift+Ctrl+Q"),
                window_submenu(vec![close_window().shortcut("Shift+Ctrl+W")]),
            ]
        );
    }

    #[test]
    fn close_and_quit_follow_the_apps_keys() {
        assert_eq!(
            Keys::Standard.standard(&key('w', false)),
            Some(Standard::CloseWindow)
        );
        assert_eq!(
            Keys::Standard.standard(&key('q', false)),
            Some(Standard::Quit)
        );
        assert_eq!(Keys::Standard.standard(&key('w', true)), None);
        assert_eq!(Keys::Shifted.standard(&key('w', false)), None);
        assert_eq!(Keys::Shifted.standard(&key('q', false)), None);
        assert_eq!(
            Keys::Shifted.standard(&key('w', true)),
            Some(Standard::CloseWindow)
        );
        assert_eq!(
            Keys::Shifted.standard(&key('q', true)),
            Some(Standard::Quit)
        );
    }
}
