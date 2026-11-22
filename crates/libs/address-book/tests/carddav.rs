#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use bande_address_book::store::{Account, AccountKind, BookInfo, Found, Library};
use bande_address_book::sync::{apply, sync_book};
use bande_address_book::vcard::{Card, Contact, Label, Labelled, display_name};
use bande_dav::{Client, Credentials, Error, Precondition, Service};

struct Radicale {
    child: Child,
    port: u16,
}

impl Drop for Radicale {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn radicale(dir: &Path) -> Option<Radicale> {
    let installed = Command::new("python3")
        .args(["-c", "import radicale"])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !installed {
        eprintln!("skipped: python3-radicale is not installed");
        return None;
    }
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    std::fs::write(dir.join("users"), "jane:secret\n").unwrap();
    let config = dir.join("config");
    std::fs::write(
        &config,
        format!(
            "[server]\nhosts = 127.0.0.1:{port}\n[auth]\ntype = htpasswd\nhtpasswd_filename = {}\n\
             htpasswd_encryption = plain\n[rights]\ntype = owner_only\n[storage]\n\
             filesystem_folder = {}\n[logging]\nlevel = warning\n",
            dir.join("users").display(),
            dir.join("collections").display()
        ),
    )
    .unwrap();
    let child = Command::new("python3")
        .args(["-m", "radicale", "--config"])
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = Radicale { child, port };
    let deadline = Instant::now() + Duration::from_secs(20);
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "radicale did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    Some(server)
}

fn make_address_book(port: u16, path: &str) {
    let body = "<?xml version=\"1.0\"?><mkcol xmlns=\"DAV:\" xmlns:card=\"urn:ietf:params:xml:ns:carddav\">\
        <set><prop><resourcetype><collection/><card:addressbook/></resourcetype>\
        <displayname>Friends</displayname></prop></set></mkcol>";
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "MKCOL {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Basic amFuZTpzZWNyZXQ=\r\n\
         Content-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut reply = String::new();
    stream.read_to_string(&mut reply).unwrap();
    assert!(reply.split_whitespace().nth(1) == Some("201"), "{reply}");
}

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
    let Some(server) = radicale(dir.path()) else {
        return;
    };
    let url = format!("http://127.0.0.1:{}/", server.port);
    let wrong = Credentials {
        user: "jane".to_owned(),
        password: "wrong".to_owned(),
    };
    let refused = Client::new(Service::CardDav, Some(&wrong)).discover_address_books(&url);
    assert_eq!(refused, Err(Error::Unauthorized));
    let credentials = Credentials {
        user: "jane".to_owned(),
        password: "secret".to_owned(),
    };
    let client = Client::new(Service::CardDav, Some(&credentials));
    assert!(client.discover_address_books(&url).is_err());
    make_address_book(server.port, "/jane/friends/");
    let found = client.discover_address_books(&url).unwrap();
    assert_eq!(found.books[0].name, "Friends");

    let mut library = Library::load(dir.path().join("store"));
    library.add_account(
        Account {
            id: "dav".to_owned(),
            name: "jane".to_owned(),
            kind: AccountKind::CardDav {
                server: url.clone(),
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
