#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bande_secrets::pam;
use bande_secrets::service::{Daemon, SERVICE_PATH, WireSecret, collection_path, item_path};
use bande_secrets::store::{CREATOR, Keychain};
use bande_test_bus::SessionBus;
use oo7::{Key, Secret, crypto};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.Secret.Service";
const EXTENSION: &str = "org.bande.Secrets";
const ITEM: &str = "org.freedesktop.Secret.Item";
const PROMPT: &str = "org.freedesktop.Secret.Prompt";
const PASSWORD: &str = "correct horse";

fn call<R: serde::de::DeserializeOwned + zbus::zvariant::Type>(
    client: &Connection,
    path: &str,
    interface: &str,
    method: &str,
    body: &(impl serde::Serialize + zbus::zvariant::DynamicType),
) -> R {
    client
        .call_method(
            Some("org.freedesktop.secrets"),
            path,
            Some(interface),
            method,
            body,
        )
        .unwrap()
        .body()
        .deserialize()
        .unwrap()
}

#[test]
fn an_app_keeps_a_secret_over_an_encrypted_session_and_cannot_forge_its_owner() {
    let bus = SessionBus::start().unwrap();
    let home = tempfile::tempdir().unwrap();
    let (daemon, _connection) = serve(&bus, home.path(), PathBuf::from("/nonexistent"));
    assert!(async_io::block_on(daemon.login(Secret::text(PASSWORD))));
    let client = bus.connect().unwrap();

    let private = Key::generate_private_key().unwrap();
    let public = Key::generate_public_key(&private).unwrap();
    let (output, session): (OwnedValue, OwnedObjectPath) = call(
        &client,
        SERVICE_PATH,
        SERVICE,
        "OpenSession",
        &(
            "dh-ietf1024-sha256-aes128-cbc-pkcs7",
            Value::from(public.as_ref().to_vec()),
        ),
    );
    let aes =
        Key::generate_aes_key(&private, &Key::new(Vec::<u8>::try_from(output).unwrap())).unwrap();
    let iv = crypto::generate_iv().unwrap();
    let sealed = crypto::encrypt(b"hunter2", &aes, &iv).unwrap();
    let attributes = HashMap::from([("service", "mail"), (CREATOR, "/usr/bin/someone-else")]);
    let properties = HashMap::from([
        ("org.freedesktop.Secret.Item.Label", Value::from("Mail")),
        (
            "org.freedesktop.Secret.Item.Attributes",
            Value::from(attributes),
        ),
    ]);
    let secret: WireSecret = (session.clone(), iv, sealed, "text/plain".to_owned());
    let (item, _): (OwnedObjectPath, OwnedObjectPath) = call(
        &client,
        collection_path("login").as_str(),
        "org.freedesktop.Secret.Collection",
        "CreateItem",
        &(properties, secret, true),
    );

    let (unlocked, locked) = search(&client, ("service", "mail"));
    assert_eq!(
        (unlocked, locked),
        (vec![item.clone()], vec![]),
        "the saving app reads its own item"
    );
    let (_, iv, sealed, _) = read_secret(&client, &item, &session).unwrap();
    assert_eq!(&*crypto::decrypt(sealed, &aes, iv).unwrap(), b"hunter2");
    let stored: OwnedValue = call(
        &client,
        item.as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        &(ITEM, "Attributes"),
    );
    let stored = HashMap::<String, String>::try_from(stored).unwrap();
    assert_ne!(
        stored.get(CREATOR).map(String::as_str),
        Some("/usr/bin/someone-else"),
        "a client cannot name another app as the owner"
    );
}

fn prompter(home: &Path, script: &str) -> PathBuf {
    let path = home.join("prompter");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\necho shown >> '{}/shown'\nwhile read -r line && [ -n \"$line\" ]; do :; done\n{script}\n",
            home.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn times_shown(home: &Path) -> usize {
    std::fs::read_to_string(home.join("shown"))
        .map(|shown| shown.lines().count())
        .unwrap_or_default()
}

fn another_apps_item(home: &Path) -> OwnedObjectPath {
    async_io::block_on(async {
        let daemon = Daemon::new(
            Keychain::load(home.to_path_buf()).await,
            PathBuf::from("/nonexistent"),
        );
        assert!(daemon.login(Secret::text(PASSWORD)).await);
        daemon
            .store_internal(
                "Home Wi-Fi",
                HashMap::from([("network".to_owned(), "home".to_owned())]),
                Secret::text("wifi-secret"),
            )
            .await;
    });
    item_path("login", 1)
}

fn serve(bus: &SessionBus, home: &Path, prompter: PathBuf) -> (Arc<Daemon>, zbus::Connection) {
    async_io::block_on(async {
        let daemon = Daemon::new(Keychain::load(home.to_path_buf()).await, prompter);
        let connection = zbus::connection::Builder::address(bus.address())
            .unwrap()
            .build()
            .await
            .unwrap();
        daemon.serve(&connection).await.unwrap();
        (daemon, connection)
    })
}

fn answer_prompt(client: &Connection, prompt: OwnedObjectPath) -> (bool, OwnedValue) {
    let proxy =
        zbus::blocking::Proxy::new(client, "org.freedesktop.secrets", prompt, PROMPT).unwrap();
    let mut completed = proxy.receive_signal("Completed").unwrap();
    proxy.call_method("Prompt", &("")).unwrap();
    completed.next().unwrap().body().deserialize().unwrap()
}

fn reveal(client: &Connection) -> (bool, OwnedValue) {
    let prompt = call(client, SERVICE_PATH, EXTENSION, "Reveal", &());
    answer_prompt(client, prompt)
}

fn search(
    client: &Connection,
    attribute: (&str, &str),
) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
    let query = HashMap::from([attribute]);
    call(client, SERVICE_PATH, SERVICE, "SearchItems", &(query,))
}

fn plain_session(client: &Connection) -> OwnedObjectPath {
    let (_, session): (OwnedValue, OwnedObjectPath) = call(
        client,
        SERVICE_PATH,
        SERVICE,
        "OpenSession",
        &("plain", Value::from("")),
    );
    session
}

fn read_secret(
    client: &Connection,
    item: &OwnedObjectPath,
    session: &OwnedObjectPath,
) -> zbus::Result<WireSecret> {
    client
        .call_method(
            Some("org.freedesktop.secrets"),
            item.as_str(),
            Some(ITEM),
            "GetSecret",
            &(session,),
        )?
        .body()
        .deserialize()
}

fn collection_locked(client: &Connection) -> bool {
    let locked: OwnedValue = call(
        client,
        collection_path("login").as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        &("org.freedesktop.Secret.Collection", "Locked"),
    );
    bool::try_from(locked).unwrap()
}

#[test]
fn another_apps_item_opens_only_with_the_keychain_password_and_until_settings_locks() {
    let bus = SessionBus::start().unwrap();
    let home = tempfile::tempdir().unwrap();
    let item = another_apps_item(home.path());
    let script = "printf 'allow\\twrong\\n'\nread -r verdict\n[ \"$verdict\" = wrong ] && printf 'allow\\tcorrect horse\\n'\nread -r verdict";
    let (_daemon, _connection) = serve(&bus, home.path(), prompter(home.path(), script));
    let client = bus.connect().unwrap();
    assert!(collection_locked(&client));

    let (dismissed, _) = reveal(&client);

    assert!(!dismissed, "the right password after a wrong one unlocks");
    assert!(!collection_locked(&client));
    assert_eq!(times_shown(home.path()), 1, "one password prompt, not two");
    let session = plain_session(&client);
    assert_eq!(
        read_secret(&client, &item, &session).unwrap().2,
        b"wifi-secret"
    );

    let () = call(&client, SERVICE_PATH, EXTENSION, "Conceal", &());
    assert!(
        read_secret(&client, &item, &session).is_err(),
        "locking the pane ends this process's access to another app's item"
    );
    assert!(
        !collection_locked(&client),
        "the keychain itself stays unlocked"
    );

    let (unlocked, locked) = search(&client, ("network", "home"));
    assert!(unlocked.is_empty(), "another app's item starts locked");
    let (opened, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        call(&client, SERVICE_PATH, SERVICE, "Unlock", &(&locked,));
    assert!(opened.is_empty());
    let (dismissed, _) = answer_prompt(&client, prompt);
    assert!(!dismissed, "the right password after a wrong one allows it");
    assert_eq!(
        read_secret(&client, &item, &session).unwrap().2,
        b"wifi-secret"
    );
}

#[test]
fn an_unlock_that_is_cancelled_or_cannot_be_asked_opens_nothing() {
    let bus = SessionBus::start().unwrap();
    let home = tempfile::tempdir().unwrap();
    let item = another_apps_item(home.path());
    let _cancelling = serve(
        &bus,
        home.path(),
        prompter(home.path(), "printf 'cancel\\n'"),
    );
    let client = bus.connect().unwrap();

    let (dismissed, result) = reveal(&client);
    assert!(dismissed);
    assert!(
        String::try_from(result).is_err(),
        "cancelling is no failure"
    );
    assert!(collection_locked(&client));
    assert!(read_secret(&client, &item, &plain_session(&client)).is_err());

    let bus = SessionBus::start().unwrap();
    let _unable = serve(&bus, home.path(), PathBuf::from("/nonexistent"));
    let client = bus.connect().unwrap();
    let (dismissed, result) = reveal(&client);
    assert!(dismissed);
    assert!(
        String::try_from(result).is_ok(),
        "a prompt that cannot appear is reported, not taken for a cancel"
    );
    assert!(collection_locked(&client));
}

const ORDINARY_CLIENT: &str = "BANDE_SECRETS_TEST_LOGIN_SOCKET";

fn send_login_message(socket: &Path) -> Vec<u8> {
    let mut stream = UnixStream::connect(socket).unwrap();
    let password = PASSWORD.as_bytes();
    let mut message = vec![1];
    message.extend_from_slice(&(password.len() as u32).to_le_bytes());
    message.extend_from_slice(password);
    let _ = stream.write_all(&message);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    answer
}

fn send_as_an_ordinary_process(socket: &Path) -> Vec<u8> {
    if !rustix::process::geteuid().is_root() {
        return send_login_message(socket);
    }
    use std::os::unix::process::CommandExt;
    let nobody = 65534;
    let place = tempfile::tempdir().unwrap();
    std::fs::set_permissions(place.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::set_permissions(
        socket.parent().unwrap(),
        std::fs::Permissions::from_mode(0o711),
    )
    .unwrap();
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666)).unwrap();
    let client = place.path().join("client");
    std::fs::copy(std::env::current_exe().unwrap(), &client).unwrap();
    std::fs::set_permissions(&client, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = std::process::Command::new(&client)
        .args([
            "--exact",
            "an_ordinary_process_sends_a_login_message",
            "--nocapture",
        ])
        .env(ORDINARY_CLIENT, socket)
        .current_dir("/")
        .gid(nobody)
        .uid(nobody)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (_, answer) = stdout
        .split_once("answer:[")
        .expect("the ordinary process reports its answer");
    let (answer, _) = answer.split_once(']').unwrap();
    answer
        .split(',')
        .filter(|byte| !byte.is_empty())
        .map(|byte| byte.parse().unwrap())
        .collect()
}

#[test]
fn an_ordinary_process_sends_a_login_message() {
    let Some(socket) = std::env::var_os(ORDINARY_CLIENT) else {
        return;
    };
    assert!(!rustix::process::geteuid().is_root());
    let answer = send_login_message(Path::new(&socket));
    let answer: Vec<String> = answer.iter().map(u8::to_string).collect();
    println!("answer:[{}]", answer.join(","));
}

#[test]
fn only_root_can_pass_a_login_password_and_a_wrong_old_password_changes_nothing() {
    let bus = SessionBus::start().unwrap();
    let home = tempfile::tempdir().unwrap();
    another_apps_item(home.path());
    let (daemon, connection) = serve(&bus, home.path(), PathBuf::from("/nonexistent"));
    let socket = home.path().join("pam");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    connection
        .executor()
        .spawn(
            {
                let (daemon, connection) = (daemon.clone(), connection.clone());
                async move {
                    let _ = pam::serve(daemon, connection, listener).await;
                }
            },
            "login socket",
        )
        .detach();

    let answer = send_as_an_ordinary_process(&socket);
    assert!(answer.is_empty(), "an ordinary process gets no answer");
    let client = bus.connect().unwrap();
    assert!(collection_locked(&client));

    async_io::block_on(async {
        assert!(daemon.login(Secret::text(PASSWORD)).await);
        assert!(
            !daemon
                .change_login(Secret::text("a guess"), Secret::text("attacker"))
                .await,
            "an open keychain still needs its old password"
        );
        let restarted = Daemon::new(
            Keychain::load(home.path().to_path_buf()).await,
            PathBuf::from("/nonexistent"),
        );
        assert!(!restarted.login(Secret::text("attacker")).await);
        assert!(restarted.login(Secret::text(PASSWORD)).await);
    });
}
