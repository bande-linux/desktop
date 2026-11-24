#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionRequest, TreeId};
use bande_discovery::testing::{Record, nameserver};
use bande_mail::compose::{Draft, Outgoing, from_mailto};
use bande_mail::imap::Session;
use bande_mail::message::Address;
use bande_mail::net::Network;
use bande_mail::setup::{Outcome, Problem, check};
use bande_mail::store::{Account, Store};
use bande_mail::worker::{Command as Order, Event, Mailroom, Status, Worker};
use bande_mail_core::guide::{Done, Guide, Host, Options};
use bande_mail_core::sheet::Mode;
use bande_mail_core::testing::{Dovecot, PASSWORD, USER, local};
use bande_toolkit::ui::{Context, Event as Input, FreeLayout, Key, Modifiers, Size, Ui, Widget};
use bande_toolkit::{Palette, Rect, Scheme, TextSystem};

const WAIT: Duration = Duration::from_secs(20);

type Sent = Arc<Mutex<Vec<(Vec<String>, String)>>>;

fn person(email: &str) -> Address {
    Address {
        name: String::new(),
        email: email.to_owned(),
    }
}

fn smtp_listener(ehlo: &'static str, sent: Sent) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut writer = stream.try_clone().unwrap();
            let mut lines = BufReader::new(stream).lines().map_while(Result::ok);
            let mut recipients = Vec::new();
            writer.write_all(b"220 ready\r\n").unwrap();
            while let Some(line) = lines.next() {
                let reply = match line.split([' ', ':']).next().unwrap_or_default() {
                    "EHLO" => ehlo,
                    "RCPT" => {
                        recipients.push(line.trim_start_matches("RCPT TO:").to_owned());
                        "250 ok\r\n"
                    }
                    "DATA" => {
                        writer.write_all(b"354 go on\r\n").unwrap();
                        let data = lines.by_ref().take_while(|line| line != ".");
                        let data = data.collect::<Vec<_>>().join("\r\n");
                        sent.lock().unwrap().push((recipients.clone(), data));
                        "250 queued\r\n"
                    }
                    "QUIT" => "221 bye\r\n",
                    _ => "250 ok\r\n",
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
fn the_setup_guide_asks_before_signing_in_off_domain_and_shows_each_problem_at_its_field() {
    if !Dovecot::installed() {
        eprintln!("dovecot is not installed; skipping");
        return;
    }
    let dovecot = Dovecot::start().unwrap();
    let smtp = smtp_listener("250 ok\r\n", Sent::default());
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
    guide.place(&mut ui, Rect::new(0.0, 0.0, 900.0, 700.0));
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
    ui.handle(&Input::Commit("Ann".to_owned()));
    for text in [USER, "wrong"] {
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
    click(&mut ui, "Cancel");
    assert!(
        guide.act(&mut ui).is_none(),
        "Cancel keeps the inline guide"
    );
    assert_eq!(*guide.mode(), Mode::Address);
    key(&mut ui, Key::Enter);
    assert!(guide.act(&mut ui).is_none());
    wait(&mut ui, &mut guide, &|mode| *mode != Mode::Checking);
    assert!(matches!(guide.mode(), Mode::Confirm(_)));

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
    token_only.smtp = local(smtp_listener(
        "250-localhost\r\n250 AUTH XOAUTH2 OAUTHBEARER\r\n",
        Sent::default(),
    ));
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

#[test]
fn a_message_goes_out_once_without_injected_headers_after_the_imap_session_went_stale() {
    if !Dovecot::installed() {
        eprintln!("dovecot is not installed; skipping");
        return;
    }
    let dovecot = Dovecot::start().unwrap();
    let sent = Sent::default();
    let path = dovecot.folder().join("mail.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut account = Account {
        id: 0,
        name: "Ann".to_owned(),
        email: USER.to_owned(),
        user: USER.to_owned(),
        imap: local(dovecot.port()),
        smtp: local(smtp_listener("250 ok\r\n", sent.clone())),
        signature: String::new(),
        enabled: true,
        description: String::new(),
    };
    account.id = store.add_account(&account).unwrap();
    let id = account.id;
    let mut mailroom = Mailroom::new().unwrap();
    let password = Box::new(|| Ok(Some(PASSWORD.to_owned())));
    let worker = Worker::spawn(
        account,
        path,
        password,
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
