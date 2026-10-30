use unbound_app_menu::{Entry, app_submenu, close_window, window_submenu};
use unbound_files::L10N;
use unbound_toolkit::ui::{Event, Key, Modifiers};

const KEY_ACTION: &str = "key:";

fn shortcut(message: &str, keys: &str) -> Entry {
    Entry::item(&L10N.tr(message), &format!("{KEY_ACTION}{keys}")).shortcut(keys)
}

fn submenu(message: &str, entries: Vec<Entry>) -> Entry {
    Entry::submenu(&L10N.tr(message), entries)
}

pub fn app_menus() -> Vec<Entry> {
    vec![
        app_submenu(&L10N.tr("app-name"), Vec::new()),
        submenu(
            "menu-file",
            vec![
                shortcut("menu-new-folder", "Shift+Super+N"),
                shortcut("menu-open", "Super+O"),
                Entry::separator(),
                shortcut("menu-get-info", "Super+I"),
                shortcut("menu-rename", "F2"),
                shortcut("menu-duplicate", "Super+D"),
                Entry::separator(),
                shortcut("menu-move-to-trash", "Super+Backspace"),
                shortcut("menu-delete-immediately", "Alt+Super+Backspace"),
                Entry::separator(),
                shortcut("menu-eject", "Super+E"),
                shortcut("menu-find", "Super+F"),
                Entry::separator(),
                close_window(),
            ],
        ),
        submenu(
            "menu-edit",
            vec![
                shortcut("menu-undo", "Super+Z"),
                shortcut("menu-redo", "Shift+Super+Z"),
                Entry::separator(),
                shortcut("menu-copy", "Super+C"),
                shortcut("menu-paste", "Super+V"),
                shortcut("menu-move-item-here", "Alt+Super+V"),
                shortcut("menu-select-all", "Super+A"),
            ],
        ),
        submenu(
            "menu-view",
            vec![
                shortcut("menu-as-columns", "Super+3"),
                shortcut("menu-as-list", "Super+2"),
                Entry::separator(),
                shortcut("menu-show-hidden-files", "Shift+Super+."),
                shortcut("menu-quick-look", "Space"),
            ],
        ),
        submenu(
            "menu-go",
            vec![
                shortcut("menu-back", "Super+["),
                shortcut("menu-forward", "Super+]"),
                shortcut("menu-enclosing-folder", "Super+Up"),
                Entry::separator(),
                shortcut("menu-desktop", "Shift+Super+D"),
                shortcut("menu-documents", "Shift+Super+O"),
                shortcut("menu-downloads", "Alt+Super+L"),
                shortcut("menu-home", "Shift+Super+H"),
                Entry::separator(),
                shortcut("menu-go-to-folder", "Shift+Super+G"),
            ],
        ),
        window_submenu(Vec::new()),
    ]
}

pub fn key_event(action: &str) -> Option<Event> {
    let keys = action.strip_prefix(KEY_ACTION)?;
    let (modifiers, key) = keys.rsplit_once('+').unwrap_or(("", keys));
    let mut held = Modifiers::default();
    for modifier in modifiers.split('+').filter(|modifier| !modifier.is_empty()) {
        match modifier {
            "Shift" => held.shift = true,
            "Super" => held.logo = true,
            "Alt" => held.alt = true,
            "Control" => held.control = true,
            _ => return None,
        }
    }
    let key = match key {
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Backspace" => Key::Backspace,
        "Space" => Key::Space,
        "F2" => Key::Function(2),
        _ => {
            let mut characters = key.chars();
            match (characters.next(), characters.next()) {
                (Some(character), None) => Key::Character(character.to_ascii_lowercase()),
                _ => return None,
            }
        }
    };
    Some(Event::Key {
        key,
        modifiers: held,
    })
}
