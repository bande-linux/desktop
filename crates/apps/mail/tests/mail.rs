#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use bande_mail::compose::{Draft, Outgoing, from_mailto};
use bande_mail::imap::{Fetched, Folder, Session};
use bande_mail::message::Address;
use bande_mail::net::{Network, Security, Server};
use bande_mail::offer::{DOMAIN, MESSAGE_ARGUMENT, offer_to_search};
use bande_mail::store::{Account, Store};
use bande_mail::worker::{Command as Order, Event, Mailroom, Status, Worker};
use bande_search::Offer;
use zbus::blocking::Connection;

const DOVECOT: &str = "/usr/sbin/dovecot";
const USER: &str = "ann@bande.test";
const PASSWORD: &str = "dovecot test";
const WAIT: Duration = Duration::from_secs(20);

type Sent = Arc<Mutex<Vec<(Vec<String>, String)>>>;

fn local(port: u16) -> Server {
    Server {
        host: "127.0.0.1".to_owned(),
        port,
        security: Security::Plain,
    }
}

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
    }
}

fn person(email: &str) -> Address {
    Address {
        name: String::new(),
        email: email.to_owned(),
    }
}

struct Dovecot {
    folder: tempfile::TempDir,
    port: u16,
}

impl Dovecot {
    fn start() -> Self {
        let folder = tempfile::Builder::new()
            .prefix("dc")
            .tempdir_in("/tmp")
            .unwrap();
        let port = TcpListener::bind("127.0.0.1:0").and_then(|probe| probe.local_addr());
        let port = port.unwrap().port();
        let user = std::env::var("USER").unwrap_or_else(|_| "root".to_owned());
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        let dir = folder.path().display();
        let config = format!(
            "dovecot_config_version = 2.4.1\ndovecot_storage_version = 2.4.1\n\
             base_dir = {dir}/run\nstate_dir = {dir}/state\nlog_path = {dir}/log\n\
             default_internal_user = {user}\ndefault_internal_group = {user}\n\
             default_login_user = {user}\nprotocols = imap\nlisten = 127.0.0.1\nssl = no\n\
             auth_allow_cleartext = yes\nmail_driver = maildir\nmail_home = {dir}/mail\n\
             mail_path = ~/Maildir\npassdb static {{\n password = {PASSWORD}\n}}\n\
             userdb static {{\n fields {{\n  uid = {uid}\n  gid = {gid}\n }}\n}}\n\
             service imap-login {{\n chroot =\n inet_listener imap {{\n  port = {port}\n }}\n\
             inet_listener imaps {{\n  port = 0\n }}\n}}\n\
             service anvil {{\n chroot =\n}}\nnamespace inbox {{\n inbox = yes\n\
             mailbox Sent {{\n  special_use = \\Sent\n  auto = subscribe\n }}\n}}\n"
        );
        std::fs::write(folder.path().join("dovecot.conf"), config).unwrap();
        let dovecot = Self { folder, port };
        assert!(dovecot.command(DOVECOT, &[]).status().unwrap().success());
        let deadline = Instant::now() + WAIT;
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "dovecot never listened");
            std::thread::sleep(Duration::from_millis(50));
        }
        dovecot
    }

    fn command(&self, program: &str, arguments: &[&str]) -> Command {
        let mut command = Command::new(program);
        command
            .arg("-c")
            .arg(self.folder.path().join("dovecot.conf"))
            .args(arguments);
        command
    }
}

impl Drop for Dovecot {
    fn drop(&mut self) {
        let _ = self.command(DOVECOT, &["stop"]).status();
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
    if !Path::new(DOVECOT).exists() {
        eprintln!("{DOVECOT} is not installed; skipping");
        return;
    }
    let dovecot = Dovecot::start();
    let sent = Sent::default();
    let path = dovecot.folder.path().join("mail.sqlite3");
    let store = Store::open(&path).unwrap();
    let mut account = account(USER, local(dovecot.port), local(smtp_server(sent.clone())));
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

    let mut session = Session::connect(&local(dovecot.port), &Network::default()).unwrap();
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
