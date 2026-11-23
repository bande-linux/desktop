use bande_app_menu::{Entry, edit_submenu};
use bande_l10n::{Text, text, tr};
use bande_mail::ui::{MailView, SHORTCUTS};

fn item(message: Text, action: &str) -> Entry {
    let entry = Entry::item(&message.tr(), action);
    match SHORTCUTS.iter().find(|(known, ..)| *known == action) {
        Some((_, shortcut, _)) => entry.shortcut(shortcut),
        None => entry,
    }
}

pub const VIEWER: &str = "viewer";

pub fn window_menu() -> Vec<Entry> {
    vec![Entry::item(&tr!("Message Viewer"), VIEWER)]
}

pub struct Shown {
    pub viewer: bool,
    pub windowed: bool,
    pub composing: bool,
}

pub fn message_menus(printable: bool) -> Vec<Entry> {
    vec![
        Entry::submenu(
            &tr!("File"),
            vec![
                item(text!("New Message"), "compose"),
                Entry::separator(),
                item(text!("Add Account…"), "add-account"),
                Entry::separator(),
                bande_app_menu::print_item(printable),
            ],
        ),
        edit_submenu(Vec::new()),
        Entry::submenu(&tr!("Mailbox"), vec![item(text!("Get Mail"), "get-mail")]),
    ]
}

pub fn app_menus(view: &MailView, shown: Shown) -> Vec<Entry> {
    let Shown {
        viewer,
        windowed,
        composing,
    } = shown;
    let targets = view
        .move_targets()
        .into_iter()
        .map(|(name, id)| Entry::item(&name, &format!("move-to:{id}")))
        .collect::<Vec<_>>();
    let selection = |message: Text, action: &str| item(message, action).enabled(viewer);
    vec![
        Entry::submenu(
            &tr!("File"),
            vec![
                item(text!("New Message"), "compose"),
                Entry::separator(),
                item(text!("Add Account…"), "add-account"),
                Entry::separator(),
                selection(text!("Find"), "find"),
                Entry::separator(),
                bande_app_menu::print_item(viewer),
            ],
        ),
        edit_submenu(Vec::new()),
        Entry::submenu(
            &tr!("View"),
            vec![
                item(text!("Keyboard Shortcuts"), "shortcuts"),
                Entry::separator(),
                item(
                    match view.sidebar_shown() {
                        true => text!("Hide Sidebar"),
                        false => text!("Show Sidebar"),
                    },
                    "toggle-sidebar",
                ),
                Entry::separator(),
                Entry::item(&tr!("Bcc Address Field"), "bcc-field")
                    .check(view.settings().bcc_field),
            ],
        ),
        Entry::submenu(
            &tr!("Mailbox"),
            vec![
                item(text!("Get Mail"), "get-mail"),
                Entry::separator(),
                selection(text!("Mark All as Read"), "mark-all-read"),
                Entry::separator(),
                item(text!("New Mailbox…"), "new-mailbox"),
            ],
        ),
        Entry::submenu(
            &tr!("Message"),
            vec![
                item(text!("Send"), "send").enabled(windowed),
                item(text!("Discard Draft"), "discard-draft").enabled(composing),
                Entry::separator(),
                selection(text!("Reply"), "reply"),
                selection(text!("Reply All"), "reply-all"),
                selection(text!("Forward"), "forward"),
                Entry::separator(),
                selection(text!("Mark as Read or Unread"), "toggle-read"),
                selection(text!("Flag or Unflag"), "toggle-flag"),
                Entry::submenu(
                    &tr!("Flag"),
                    MailView::flag_colors()
                        .into_iter()
                        .map(|(name, action)| Entry::item(&name, &action))
                        .collect(),
                )
                .enabled(viewer),
                Entry::separator(),
                selection(text!("Apply Rules"), "apply-rules"),
                Entry::separator(),
                selection(text!(context = "action", "Archive"), "archive"),
                selection(text!("Move to Junk"), "junk"),
                selection(text!("Move To…"), "move-to"),
                Entry::submenu(&tr!("Move To"), targets).enabled(viewer),
                selection(text!("Delete"), "delete"),
            ],
        ),
    ]
}
