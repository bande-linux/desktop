#![allow(clippy::unwrap_used)]

use bande_address_book::store::{Account, AccountKind, BookInfo, Found, Library};
use bande_address_book::sync::{apply, sync_book};
use bande_address_book::vcard::{Card, Contact, Label, Labelled, display_name};
use bande_dav::{Client, Precondition, Service};
use bande_test_bus::xandikos::Xandikos;

fn ann() -> Contact {
    let mut contact = Contact {
        organisation: "Fjord Partners".to_owned(),
        ..Contact::default()
    };
    contact.name.given = "Ann".to_owned();
    contact.name.family = "Lindqvist".to_owned();
    contact.emails.push(Labelled {
        label: Label::Work,
        value: "ann@fjord.example".to_owned(),
    });
    contact
}

fn sync(client: &Client, library: &mut Library) -> Vec<String> {
    let outcome = sync_book(client, &library.books[1].for_sync()).unwrap();
    apply(library, outcome)
}

#[test]
fn an_offline_edit_that_meets_a_change_on_the_carddav_server_keeps_both_versions() {
    let dir = tempfile::tempdir().unwrap();
    let Some(server) = Xandikos::start().unwrap() else {
        return;
    };
    let client = Client::new(Service::CardDav, None);
    let found = client.discover_address_books(server.url()).unwrap();

    let mut library = Library::load(dir.path().join("store"));
    library.add_account(
        Account {
            id: "dav".to_owned(),
            name: "jane".to_owned(),
            kind: AccountKind::CardDav {
                server: server.url().to_owned(),
                user: "jane".to_owned(),
                home: found.home.clone(),
            },
        },
        vec![BookInfo::remote("dav", &found.books[0])],
    );
    library.put_card(1, Card::new("ann", &ann())).unwrap();
    assert!(sync(&client, &mut library).is_empty());
    let stored = library.books[1].cards[0].clone();
    assert_eq!(stored.pending, None);

    let mut elsewhere = ann();
    elsewhere.phones.push(Labelled {
        label: Label::Mobile,
        value: "+46 70 123 45 67".to_owned(),
    });
    let theirs = Card::new("ann", &elsewhere);
    client
        .put(
            &stored.href,
            &theirs.to_vcf(),
            &Precondition::Match(stored.etag.clone().unwrap()),
        )
        .unwrap();
    let mut mine = ann();
    mine.note = "Prefers mornings".to_owned();
    library.edit(&Found { book: 1, card: 0 }, &mine).unwrap();

    assert_eq!(sync(&client, &mut library), ["Ann Lindqvist"]);
    assert!(sync(&client, &mut library).is_empty());
    let shown: Vec<Contact> = library
        .contacts()
        .into_iter()
        .filter(|(found, _)| found.book == 1)
        .map(|(_, card)| card.contact())
        .collect();
    assert_eq!(shown.len(), 2, "{shown:?}");
    assert!(shown.iter().any(|contact| contact.phones.len() == 1));
    assert!(
        shown
            .iter()
            .any(|contact| contact.note == "Prefers mornings")
    );
    assert!(
        shown
            .iter()
            .all(|contact| display_name(contact) == "Ann Lindqvist")
    );
    assert!(
        library.books[1]
            .cards
            .iter()
            .all(|stored| stored.pending.is_none())
    );

    client.delete(&stored.href, None).unwrap();
    sync(&client, &mut library);
    let left: Vec<String> = library.books[1]
        .cards
        .iter()
        .map(|stored| stored.href.clone())
        .collect();
    assert_eq!(left.len(), 1);
    assert_ne!(left[0], stored.href);
}
