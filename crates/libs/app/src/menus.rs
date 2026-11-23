use bande_app_menu::{
    Entry, FULL_SCREEN, Standard, WINDOW_ACTION, about_item, close_window, hide_items, quit_item,
    settings, tidy, window_submenu,
};
use bande_toolkit::ui::{Event, MenuItem, MenuToggle, shortcut_pressed};

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
            (FULL_SCREEN, Standard::FullScreen),
        ]
        .into_iter()
        .filter(|(_, standard)| settings || *standard != Standard::Settings)
        .find(|(shortcut, _)| shortcut_pressed(event, shortcut))
        .map(|(_, standard)| standard)
    }
}

fn window_entry(item: MenuItem) -> Entry {
    if item.separator {
        return Entry::separator();
    }
    let entry = match item.submenu {
        Some(menu) => Entry::submenu(
            &item.title,
            menu.items.into_iter().map(window_entry).collect(),
        ),
        None => Entry::item(&item.title, &format!("{WINDOW_ACTION}{}", item.action)),
    }
    .enabled(item.enabled);
    let entry = match item.shortcut.filter(|shortcut| !shortcut.is_empty()) {
        Some(shortcut) => entry.shortcut(&shortcut),
        None => entry,
    };
    match item.toggle {
        Some(MenuToggle::Check(on)) => entry.check(on),
        Some(MenuToggle::Radio(on)) => entry.radio(on),
        None => entry,
    }
}

pub(crate) fn assemble(
    info: &Info,
    menus: Menus,
    windowed: bool,
    commands: Vec<MenuItem>,
) -> Vec<Entry> {
    let name = (info.name)();
    let mut window: Vec<Entry> = commands.into_iter().map(window_entry).collect();
    window.push(Entry::separator());
    window.extend(menus.window);
    window.push(Entry::separator());
    window.push(close_window(info.keys.close()));
    window.push(Entry::separator());
    window.extend(hide_items(&name));
    window.push(Entry::separator());
    window.push(about_item(&name, info.version));
    let mut entries = menus.menus;
    entries.push(window_submenu(window));
    entries.push(Entry::separator());
    if info.settings.is_some() {
        entries.push(settings(info.keys.settings()));
    }
    entries.push(quit_item(&name, info.keys.quit()));
    let entries = tidy(entries);
    match windowed {
        true => entries,
        false => entries.into_iter().map(Entry::without_window).collect(),
    }
}
