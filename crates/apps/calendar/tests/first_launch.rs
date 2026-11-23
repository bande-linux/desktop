#![allow(clippy::unwrap_used)]

use std::path::Path;

use accesskit::{Action, ActionRequest, Role, TreeId};
use bande_calendar::settings::CalendarSettings;
use bande_calendar::store::{AccountKind, LOCAL_ACCOUNT, Library};
use bande_calendar::ui::CalendarView;
use bande_datetime::DateFormats;
use bande_inbox::Inbox;
use bande_toolkit::{Appearance, TextSystem};
use jiff::Zoned;

fn launch(data: &Path, remembered: &Path) -> CalendarView {
    let mut view = CalendarView::new(
        Library::load(data.to_path_buf()),
        Appearance::default().palette(),
        TextSystem::new(),
        DateFormats::from_env(),
        CalendarSettings::default(),
        Zoned::now(),
        Inbox::new().unwrap().outbox(),
    );
    view.welcome(remembered.to_path_buf());
    view
}

fn shows(view: &mut CalendarView, role: Role, name: &str) -> Option<accesskit::NodeId> {
    view.scene(1000, 700);
    view.accessibility()
        .nodes
        .into_iter()
        .find(|(_, node)| node.role() == role && node.label() == Some(name))
        .map(|(id, _)| id)
}

fn click(view: &mut CalendarView, name: &str) {
    let button = shows(view, Role::Button, name).unwrap();
    view.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: button,
        data: None,
    });
}

#[test]
fn the_first_launch_offers_an_account_once_and_otherwise_keeps_the_calendar_on_this_computer() {
    let home = tempfile::tempdir().unwrap();
    let data = home.path().join("data");
    let remembered = home.path().join("state/calendar-welcome");

    let mut view = launch(&data, &remembered);
    click(&mut view, "Use on This Computer");
    assert!(remembered.exists());
    let library = view.library();
    assert!(
        library
            .accounts
            .iter()
            .all(|account| account.kind == AccountKind::Local)
    );
    assert!(
        library
            .calendars
            .iter()
            .any(|calendar| calendar.info.account == LOCAL_ACCOUNT && !calendar.info.read_only)
    );

    let mut again = launch(&data, &remembered);
    assert!(!again.has_sheet());
    assert!(shows(&mut again, Role::Button, "Add Account…").is_none());

    let elsewhere = home.path().join("other/calendar-welcome");
    let mut view = launch(&data, &elsewhere);
    click(&mut view, "Add Account…");
    assert!(elsewhere.exists());
    assert!(shows(&mut view, Role::Button, "Use on This Computer").is_none());
    assert!(shows(&mut view, Role::Button, "Cancel").is_some());
}
