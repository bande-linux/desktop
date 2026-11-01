use bande_app_menu::{Entry, Standard, app_submenu, close_window, settings, window_submenu};
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

pub(crate) fn assemble(info: &Info, menus: Menus, windowed: bool) -> Vec<Entry> {
    let name = (info.name)();
    let items = match info.settings {
        Some(_) => vec![settings(info.keys.settings())],
        None => Vec::new(),
    };
    let mut entries = vec![app_submenu(&name, info.version, items, info.keys.quit())];
    entries.extend(menus.menus);
    let mut window = vec![close_window(info.keys.close())];
    window.extend(menus.window);
    entries.push(window_submenu(window));
    match windowed {
        true => entries,
        false => entries.into_iter().map(Entry::without_window).collect(),
    }
}
