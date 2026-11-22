#![allow(clippy::unwrap_used, clippy::panic)]

use accesskit::{Action, ActionRequest, Node, NodeId, Role, TreeId};
use bande_address_book::store::Library;
use bande_address_book::vcard::parse_cards;
use bande_contacts::view::{ContactsView, REDO, UNDO};
use bande_toolkit::Appearance;
use bande_toolkit::ui::{Event, Key, Modifiers};

const FILE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:ann\r\nFN:Ann Lindqvist\r\n\
N:Lindqvist;Ann;;;\r\nEMAIL:ann@fjord.example\r\nEND:VCARD\r\nBEGIN:VCARD\r\nVERSION:3.0\r\n\
UID:bo\r\nFN:Bo Ek\r\nN:Ek;Bo;;;\r\nCATEGORIES:Family\r\nEND:VCARD\r\n";

fn tree(view: &mut ContactsView) -> Vec<(NodeId, Node)> {
    view.ui().scene(980, 640);
    view.ui().accessibility().nodes
}

fn click(view: &mut ContactsView, name: &str) {
    let nodes = tree(view);
    let Some((id, _)) = nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Button && node.label() == Some(name))
    else {
        panic!("there is no {name} button");
    };
    view.ui().perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: *id,
        data: None,
    });
    view.pump();
}

fn typed(view: &mut ContactsView, text: &str) {
    for character in text.chars() {
        let key = match character {
            '\t' => Key::Tab,
            character => Key::Character(character),
        };
        view.ui().handle(&Event::Key {
            key,
            modifiers: Modifiers::default(),
        });
    }
    view.pump();
}

fn listed(view: &mut ContactsView) -> Vec<String> {
    let nodes = tree(view);
    let list = nodes
        .iter()
        .rfind(|(_, node)| node.role() == Role::Table)
        .map(|(_, node)| node.children().to_vec())
        .unwrap_or_default();
    nodes
        .iter()
        .filter(|(id, node)| list.contains(id) && node.role() == Role::Row)
        .map(|(_, node)| node.label().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn a_typed_contact_is_undone_and_redone_and_imported_duplicates_merge_into_one() {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::load(dir.path().to_path_buf());
    let mut view = ContactsView::new(library, Appearance::default());
    assert!(listed(&mut view).is_empty());

    click(&mut view, "New Contact");
    typed(&mut view, "Ann\tLindqvist");
    click(&mut view, "Done");
    assert_eq!(listed(&mut view), ["Ann Lindqvist"]);
    view.command(UNDO);
    assert!(listed(&mut view).is_empty());
    view.command(REDO);
    assert_eq!(listed(&mut view), ["Ann Lindqvist"]);

    let parsed = parse_cards(FILE);
    view.offer_import("friends.vcf", parsed.cards, parsed.skipped);
    click(&mut view, "Import");
    assert_eq!(
        listed(&mut view),
        ["Bo Ek", "Ann Lindqvist", "Ann Lindqvist"]
    );
    click(&mut view, "Merge Duplicates…");
    click(&mut view, "Merge");
    assert_eq!(listed(&mut view), ["Bo Ek", "Ann Lindqvist"]);

    let kept = Library::load(dir.path().to_path_buf());
    let emails: Vec<String> = kept
        .contacts()
        .iter()
        .flat_map(|(_, card)| card.contact().emails)
        .map(|email| email.value)
        .collect();
    assert_eq!(emails, ["ann@fjord.example"]);
}
