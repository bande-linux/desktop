#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use bande_app::View;
use bande_mail::net::{Security, Server};
use bande_mail::store::{Account, Store};
use bande_mail::ui::SettingsView;
use bande_toolkit::ui::Event;
use bande_toolkit::{Appearance, TextSystem};

const SIZE: (u32, u32) = (780, 640);

fn nodes(view: &mut SettingsView) -> Vec<(NodeId, Node)> {
    view.render(SIZE.0, SIZE.1);
    view.accessibility().unwrap().nodes
}

fn act(view: &mut SettingsView, action: Action, find: impl Fn(&Node) -> bool) {
    let (target, _) = nodes(view)
        .into_iter()
        .find(|(_, node)| find(node))
        .expect("no such node");
    view.perform(&ActionRequest {
        action,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    });
}

fn labels(view: &mut SettingsView) -> Vec<String> {
    nodes(view)
        .into_iter()
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

#[test]
fn each_account_has_a_settings_page_whose_description_names_it_in_the_sidebar() {
    let home = tempfile::tempdir().unwrap();
    for (variable, folder) in [
        ("HOME", ""),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_CACHE_HOME", "cache"),
        ("GNUPGHOME", "gnupg"),
    ] {
        let path = home.path().join(folder);
        std::fs::create_dir_all(&path).unwrap();
        // SAFETY: this test is alone in its binary and sets the variables before any other thread starts.
        unsafe { std::env::set_var(variable, path) };
    }
    let path = home.path().join("mail.sqlite3");
    let store = Store::open(&path).unwrap();
    let server = Server {
        host: "127.0.0.1".to_owned(),
        port: 1,
        security: Security::Plain,
    };
    let id = store
        .add_account(&Account {
            id: 0,
            name: "Ann".to_owned(),
            email: "ann@bande.test".to_owned(),
            user: "ann@bande.test".to_owned(),
            imap: server.clone(),
            smtp: server,
            signature: String::new(),
            enabled: true,
            description: String::new(),
        })
        .unwrap();
    let mut view = SettingsView::new(Ok(path), Appearance::default(), TextSystem::new());

    let shown = labels(&mut view);
    for page in [
        "General",
        "Reading",
        "Composing",
        "Privacy",
        "Rules",
        "ann@bande.test",
        "Add Account…",
    ] {
        assert!(
            shown.iter().any(|label| label == page),
            "no {page} page in {shown:?}"
        );
    }

    act(&mut view, Action::Click, |node| {
        node.label() == Some("ann@bande.test") && node.role() != Role::Label
    });
    let shown = labels(&mut view);
    assert!(
        shown.iter().any(|label| label == "Remove Account…"),
        "{shown:?}"
    );
    act(&mut view, Action::Focus, |node| {
        node.role() == Role::TextInput && node.placeholder() == Some("ann@bande.test")
    });
    view.handle(&Event::Commit("Work".to_owned()));

    assert_eq!(store.account(id).unwrap().unwrap().description, "Work");
    assert!(labels(&mut view).iter().any(|label| label == "Work"));
}
