#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, TreeId};
use bande_discovery::testing::{Record, nameserver};
use bande_mail::compose::{Draft, Outgoing, from_mailto};
use bande_mail::imap::{Fetched, Folder, Session};
use bande_mail::message::Address;
use bande_mail::net::{Network, Server};
use bande_mail::offer::{DOMAIN, MESSAGE_ARGUMENT, offer_to_search};
use bande_mail::setup::{Outcome, Problem, check};
use bande_mail::store::{Account, Store};
use bande_mail::worker::{Command as Order, Event, Mailroom, Status, Worker};
use bande_mail_core::guide::{Done, Guide, Host, Options};
use bande_mail_core::sheet::Mode;
use bande_mail_core::testing::{Dovecot, PASSWORD, USER, local};
use bande_search::Offer;
use bande_toolkit::ui::{Context, Event as Input, FreeLayout, Key, Modifiers, Size, Ui, Widget};
use bande_toolkit::{Palette, Scheme, TextSystem};
use zbus::blocking::Connection;

const WAIT: Duration = Duration::from_secs(20);
static SEARCH_ON_THE_SESSION_BUS: Mutex<()> = Mutex::new(());

type Sent = Arc<Mutex<Vec<(Vec<String>, String)>>>;

fn account(email: &str, imap: Server, smtp: Server) -> Account {
    Account {
        id: 0,
        name: "Ann".to_owned(),
        email: email.to_owned(),
        user: email.to_owned(),
        imap,
        smtp,
        signature: String::new(),
        enabled: true,
        description: String::new(),
    }
}

fn person(email: &str) -> Address {
    Address {
        name: String::new(),
        email: email.to_owned(),
    }
}

fn smtp_server(sent: Sent) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut writer = stream.try_clone().unwrap();
            let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
            let mut recipients = Vec::new();
            writer.write_all(b"220 ready\r\n").unwrap();
            while let Some(line) = lines.next() {
                let reply = match line.split(':').next().unwrap_or_default() {
                    "RCPT TO" => {
                        recipients.push(line.trim_start_matches("RCPT TO:").to_owned());
                        "250 ok"
                    }
                    "DATA" => {
                        writer.write_all(b"354 go on\r\n").unwrap();
                        let data = lines.by_ref().take_while(|line| line != ".");
                        let data = data.collect::<Vec<_>>().join("\r\n");
                        sent.lock().unwrap().push((recipients.clone(), data));
                        "250 queued"
                    }
                    "QUIT" => "221 bye",
                    _ => "250 ok",
                };
                writer.write_all(format!("{reply}\r\n").as_bytes()).unwrap();
            }
        }
    });
    port
}

#[test]
fn a_message_goes_out_once_without_injected_headers_after_the_imap_session_went_stale() {
    let _search = SEARCH_ON_THE_SESSION_BUS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if !Dovecot::installed() {
        eprintln!("dovecot is not installed; skipping");
        return;
    }
    let dovecot = Dovecot::start().unwrap();
    let sent = Sent::default();
    let path = dovecot.folder().join("mail.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut account = account(
        USER,
        local(dovecot.port()),
        local(smtp_server(sent.clone())),
    );
    account.id = store.add_account(&account).unwrap();
    let id = account.id;
    let mut mailroom = Mailroom::new().unwrap();
    let worker = Worker::spawn(
        account,
        path,
        Box::new(|| Ok(Some(PASSWORD.to_owned()))),
        Network::default(),
        mailroom.notifier(),
    );
    let deadline = Instant::now() + WAIT;
    while mailroom.wait(Duration::from_millis(100)) != Some(Event::Status(id, Status::Ready)) {
        assert!(Instant::now() < deadline, "the first sync never finished");
    }

    let kicked = dovecot.command("doveadm", &["kick", USER]).status();
    assert!(kicked.unwrap().success());
    let mut eve = person("eve@example.com");
    eve.name = "Eve\r\nBcc: spy@example.net".to_owned();
    worker.send(Order::Send(Box::new(Outgoing {
        draft: Draft {
            from: person(USER),
            to: vec![person("bob@example.com"), eve],
            bcc: vec![person("boss@example.com")],
            subject: "Plans\r\nInjected: header".to_owned(),
            body: "See you".to_owned(),
            ..Draft::default()
        },
        ..Outgoing::default()
    })));

    let mut session = Session::connect(&local(dovecot.port()), &Network::default()).unwrap();
    session.login(USER, PASSWORD).unwrap();
    let deadline = Instant::now() + WAIT;
    while session.select("Sent").unwrap().exists == 0 {
        assert!(Instant::now() < deadline, "no copy reached Sent");
        std::thread::sleep(Duration::from_millis(100));
    }
    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "the message went out more than once");
    let (recipients, data) = &sent[0];
    let everyone = [
        "<bob@example.com>",
        "<eve@example.com>",
        "<boss@example.com>",
    ];
    assert_eq!(recipients, &everyone);
    assert!(!data.contains("boss@example.com"));
    assert!(!data.contains("\r\nBcc:"));
    assert!(!data.contains("\r\nInjected:"));
}

#[test]
fn the_setup_guide_asks_before_signing_in_off_domain_and_shows_each_problem_at_its_field() {
    if !Dovecot::installed() {
        eprintln!("dovecot is not installed; skipping");
        return;
    }
    let dovecot = Dovecot::start().unwrap();
    let smtp = smtp_server(Sent::default());
    let record = |service: &str, port| Record::Srv {
        name: format!("{service}._tcp.bande.test"),
        port,
        target: "localhost".to_owned(),
    };
    let records = vec![record("_imap", dovecot.port()), record("_submission", smtp)];
    let network = Network {
        resolver: nameserver(records).unwrap(),
        ..Network::default()
    };
    let (reports, received) = std::sync::mpsc::channel();
    let layout = FreeLayout::new();
    let host = Widget::id(&layout);
    let palette = Palette::for_scheme(Scheme::Light);
    let mut ui = Ui::new(Box::new(layout), Context::new(TextSystem::new(), palette));
    let options = Options {
        icon: b"<svg xmlns='http://www.w3.org/2000/svg'/>",
        offer_notes: true,
        server_settings: true,
        ready_note: String::new(),
        taken: Vec::new(),
        network: network.clone(),
        reporter: Arc::new(move |report| {
            let _ = reports.send(report);
        }),
    };
    let mut guide = Guide::open(&mut ui, Host::Inline(host), Mode::Address, options);
    let wait = |ui: &mut Ui, guide: &mut Guide, until: &dyn Fn(&Mode) -> bool| loop {
        let report = received
            .recv_timeout(WAIT)
            .expect("the guide stopped reporting");
        let done = guide.report(ui, report);
        if done.is_some() || until(guide.mode()) {
            return done;
        }
    };

    assert!(!names(&mut ui).contains(&"Cancel".to_owned()));
    for text in ["Ann", USER, "wrong"] {
        key(&mut ui, Key::Tab);
        ui.handle(&Input::Commit(text.to_owned()));
    }
    key(&mut ui, Key::Enter);
    assert!(guide.act(&mut ui).is_none());
    wait(&mut ui, &mut guide, &|mode| *mode != Mode::Checking);
    assert!(
        matches!(guide.mode(), Mode::Confirm(_)),
        "servers outside bande.test are confirmed before the password goes there"
    );

    click(&mut ui, "Sign In");
    assert!(guide.act(&mut ui).is_none());
    wait(&mut ui, &mut guide, &|mode| *mode != Mode::Checking);
    assert_eq!(*guide.mode(), Mode::Servers);
    assert!(described(&mut ui).contains("offers no encrypted connection"));

    for choice in [0, 1] {
        click_nth(&mut ui, "Security", choice);
        ui.take_menu_request().unwrap();
        ui.menu_action("2");
    }
    click(&mut ui, "Sign In");
    assert!(guide.act(&mut ui).is_none());
    wait(&mut ui, &mut guide, &|mode| *mode != Mode::Checking);
    assert_eq!(*guide.mode(), Mode::Servers);
    assert!(described(&mut ui).contains("did not accept the user name or password"));

    ui.handle(&Input::Commit(PASSWORD.to_owned()));
    key(&mut ui, Key::Enter);
    assert!(guide.act(&mut ui).is_none());
    let Some(Done::Ready {
        settings, password, ..
    }) = wait(&mut ui, &mut guide, &|_| false)
    else {
        panic!("the right password did not sign in");
    };
    assert_eq!(
        (settings.imap.port, settings.smtp.port, password.as_str()),
        (dovecot.port(), smtp, PASSWORD)
    );

    guide.added(&mut ui, USER.to_owned());
    key(&mut ui, Key::Tab);
    ui.handle(&Input::Commit("Work".to_owned()));
    click(&mut ui, "Done");
    let Some(Done::Finished {
        description, notes, ..
    }) = guide.act(&mut ui)
    else {
        panic!("the account was not finished");
    };
    assert_eq!((description.as_str(), notes), ("Work", true));

    let mut token_only = settings;
    token_only.smtp = local(token_only_smtp());
    let refused = check(
        &token_only,
        PASSWORD,
        &network,
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(
        matches!(refused, Outcome::Failed(_, Problem::NoPasswordSignIn(_))),
        "{refused:?}"
    );
}

fn key(ui: &mut Ui, key: Key) {
    ui.handle(&Input::Key {
        key,
        modifiers: Modifiers::default(),
    });
}

fn nodes(ui: &mut Ui) -> Vec<(accesskit::NodeId, accesskit::Node)> {
    ui.layout(Size::new(900.0, 700.0));
    ui.accessibility().nodes
}

fn names(ui: &mut Ui) -> Vec<String> {
    nodes(ui)
        .iter()
        .filter_map(|(_, node)| node.label().map(str::to_owned))
        .collect()
}

fn described(ui: &mut Ui) -> String {
    nodes(ui)
        .iter()
        .filter_map(|(_, node)| node.description().map(str::to_owned))
        .collect::<Vec<_>>()
        .join("\n")
}

fn click_nth(ui: &mut Ui, name: &str, nth: usize) {
    let (target, _) = nodes(ui)
        .into_iter()
        .filter(|(_, node)| node.label() == Some(name))
        .nth(nth)
        .unwrap_or_else(|| panic!("no {name}"));
    ui.perform(&ActionRequest {
        action: Action::Click,
        target_tree: TreeId::ROOT,
        target_node: target,
        data: None,
    });
}

fn click(ui: &mut Ui, name: &str) {
    click_nth(ui, name, 0);
}

fn token_only_smtp() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut writer = stream.try_clone().unwrap();
            let lines = BufReader::new(stream).lines().map_while(Result::ok);
            writer.write_all(b"220 ready\r\n").unwrap();
            for line in lines {
                let reply = match line.starts_with("EHLO") {
                    true => "250-localhost\r\n250 AUTH XOAUTH2 OAUTHBEARER\r\n",
                    false => "221 bye\r\n",
                };
                if writer.write_all(reply.as_bytes()).is_err() {
                    break;
                }
            }
        }
    });
    port
}

#[test]
fn a_mailto_link_cannot_add_hidden_recipients_or_attachments() {
    let link =
        "mailto:ann@example.com?bcc=spy%40example.net&attach=/etc/passwd&cc=cat%40example.org";
    let draft = from_mailto(link).unwrap();
    assert_eq!(draft.to, [person("ann@example.com")]);
    assert_eq!(draft.cc, [person("cat@example.org")]);
    assert!(draft.bcc.is_empty());
    assert!(draft.attachments.is_empty());
}

#[derive(Default)]
struct Taken {
    items: Vec<Offer>,
    forgotten: Vec<String>,
    cursor: String,
}

struct Index(Arc<Mutex<Taken>>);

#[zbus::interface(name = "org.bande.Search")]
impl Index {
    fn offer(&self, domain: String, items: Vec<Offer>, cursor: String) {
        assert_eq!(domain, DOMAIN);
        let mut taken = self.0.lock().unwrap();
        taken.items.extend(items);
        taken.cursor = cursor;
    }

    fn cursor(&self, _domain: String) -> String {
        self.0.lock().unwrap().cursor.clone()
    }

    fn forget(&self, _domain: String, ids: Vec<String>) {
        self.0.lock().unwrap().forgotten.extend(ids);
    }
}

fn offer<'a>(store: &Store, taken: &'a Mutex<Taken>) -> MutexGuard<'a, Taken> {
    offer_to_search(store);
    taken.lock().unwrap()
}

fn item<'a>(taken: &'a Taken, kind: &str) -> &'a Offer {
    taken.items.iter().find(|item| item.kind == kind).unwrap()
}

#[test]
fn search_is_told_of_new_mail_its_text_and_its_removal_and_catches_up_after_losing_its_index() {
    let _search = SEARCH_ON_THE_SESSION_BUS
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let taken = Arc::new(Mutex::new(Taken::default()));
    let service = Connection::session().unwrap();
    let index = Index(Arc::clone(&taken));
    service
        .object_server()
        .at("/org/bande/Search", index)
        .unwrap();
    service.request_name("org.bande.Search").unwrap();
    let folder = tempfile::tempdir().unwrap();
    let mut store = Store::open(&folder.path().join("mail.sqlite3")).unwrap();
    let id = store
        .add_account(&account(USER, local(1), local(1)))
        .unwrap();
    let inbox = Folder {
        path: "INBOX".to_owned(),
        name: "Inbox".to_owned(),
        delimiter: None,
        role: None,
        selectable: true,
    };
    store.replace_mailboxes(id, &[inbox]).unwrap();
    let mailbox = store.mailboxes(id).unwrap()[0].id;
    let header = b"From: Ada <ada@example.org>\r\nSubject: Lunch\r\n\r\n";
    let fetched = Fetched {
        uid: 7,
        flags: None,
        size: Some(10),
        internal_date: Some(1),
        header: Some(header.to_vec()),
        body: None,
    };
    store.insert_headers(mailbox, &[fetched]).unwrap();
    let given = offer(&store, &taken);
    let mail = item(&given, "mail");
    assert_eq!(
        (mail.title.as_str(), mail.detail.as_str()),
        ("Lunch", "Ada")
    );
    assert!(mail.target.starts_with(MESSAGE_ARGUMENT));
    assert_eq!(item(&given, "contact").target, "mailto:ada@example.org");
    drop(given);

    let text = [&header[..], b"The vermilion place\r\n"].concat();
    store.store_body(mailbox, 7, &text).unwrap();
    let has_text = |taken: &Taken| {
        taken
            .items
            .iter()
            .any(|item| item.body.contains("vermilion"))
    };
    assert!(
        has_text(&offer(&store, &taken)),
        "the text reaches search with the body"
    );
    *taken.lock().unwrap() = Taken::default();
    assert!(
        has_text(&offer(&store, &taken)),
        "an index that lost everything gets it all again"
    );
    store.remove_uids(mailbox, &[7]).unwrap();
    assert_eq!(offer(&store, &taken).forgotten.len(), 1);
}

fn opened(view: &mut bande_mail::ui::MessageView) -> accesskit::TreeUpdate {
    let deadline = Instant::now() + WAIT;
    loop {
        view.arrived();
        view.scene(800, 700);
        let tree = view.accessibility();
        let opening = tree
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some("Opening the message…"));
        if !opening || Instant::now() > deadline {
            return tree;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn an_eml_file_opens_read_only_with_its_text_and_attachment() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("report.eml");
    std::fs::write(
        &path,
        "From: Ada <ada@example.org>\r\nTo: bob@example.org\r\nSubject: Quarterly report\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain\r\n\r\nSee the attached figures.\r\n\
         --b\r\nContent-Type: text/csv\r\nContent-Disposition: attachment; filename=figures.csv\r\n\r\n\
         a,b\r\n--b--\r\n",
    )
    .unwrap();
    let store = folder.path().join("mail.sqlite3");
    let open = |path: &std::path::Path| {
        bande_mail::ui::MessageView::new(
            path.to_path_buf(),
            &store,
            bande_mail::settings::MailSettings::default(),
            bande_toolkit::Appearance::default().palette(),
            bande_toolkit::TextSystem::new(),
        )
        .unwrap()
    };
    let mut view = open(&path);
    let tree = opened(&mut view);
    assert_eq!(view.title(), "Quarterly report");
    assert!(tree.nodes.iter().any(|(_, node)| {
        node.value()
            .is_some_and(|text| text.contains("See the attached figures."))
    }));
    assert!(
        tree.nodes
            .iter()
            .any(|(_, node)| node.label() == Some("figures.csv"))
    );
    view.menu_action("attachment-save-as:0");
    assert!(
        view.take_requests()
            .contains(&bande_mail::ui::Request::SaveAttachment(
                "figures.csv".to_owned(),
                b"a,b".to_vec(),
            ))
    );

    let mut missing = open(&folder.path().join("missing.eml"));
    let tree = opened(&mut missing);
    assert!(tree.nodes.iter().any(|(_, node)| {
        node.label()
            .is_some_and(|label| label.starts_with("The message couldn’t be opened"))
    }));
}
