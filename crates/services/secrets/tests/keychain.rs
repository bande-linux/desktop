#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;

use bande_secrets::service::{Daemon, SERVICE_PATH, WireSecret, collection_path};
use bande_secrets::store::{CREATOR, Keychain};
use bande_test_bus::SessionBus;
use oo7::{Key, Secret, crypto};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.Secret.Service";

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
    let daemon = async_io::block_on(async {
        let daemon = Daemon::new(
            Keychain::load(home.path().to_path_buf()).await,
            PathBuf::from("/nonexistent"),
        );
        let connection = zbus::connection::Builder::address(bus.address())
            .unwrap()
            .build()
            .await
            .unwrap();
        daemon.serve(&connection).await.unwrap();
        assert!(daemon.login(Secret::text("correct horse")).await);
        (daemon, connection)
    });
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
    let login = collection_path("login");
    let (item, _): (OwnedObjectPath, OwnedObjectPath) = call(
        &client,
        login.as_str(),
        "org.freedesktop.Secret.Collection",
        "CreateItem",
        &(properties, secret, true),
    );

    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = call(
        &client,
        SERVICE_PATH,
        SERVICE,
        "SearchItems",
        &(HashMap::from([("service", "mail")]),),
    );
    assert_eq!(
        (unlocked, locked),
        (vec![item.clone()], vec![]),
        "the saving app reads its own item"
    );
    let (_, iv, sealed, _): WireSecret = call(
        &client,
        item.as_str(),
        "org.freedesktop.Secret.Item",
        "GetSecret",
        &(&session,),
    );
    assert_eq!(&*crypto::decrypt(sealed, &aes, iv).unwrap(), b"hunter2");
    let stored: OwnedValue = call(
        &client,
        item.as_str(),
        "org.freedesktop.DBus.Properties",
        "Get",
        &("org.freedesktop.Secret.Item", "Attributes"),
    );
    let stored = HashMap::<String, String>::try_from(stored).unwrap();
    assert_ne!(
        stored.get(CREATOR).map(String::as_str),
        Some("/usr/bin/someone-else"),
        "a client cannot name another app as the owner"
    );

    let (daemon, _connection) = daemon;
    async_io::block_on(async {
        let restarted = Daemon::new(
            Keychain::load(home.path().to_path_buf()).await,
            PathBuf::from("/nonexistent"),
        );
        assert!(
            !restarted.login(Secret::text("wrong")).await,
            "a wrong password keeps the keychain shut"
        );
        assert!(restarted.login(Secret::text("correct horse")).await);
    });
    drop(daemon);
}

#[test]
fn another_apps_item_opens_only_with_the_keychain_password() {
    let bus = SessionBus::start().unwrap();
    let home = tempfile::tempdir().unwrap();
    let prompter = home.path().join("prompter");
    std::fs::write(
        &prompter,
        "#!/bin/sh\nwhile read -r line && [ -n \"$line\" ]; do :; done\nprintf 'allow\\twrong\\n'\nread -r verdict\n[ \"$verdict\" = wrong ] && printf 'allow\\tcorrect horse\\n'\nread -r verdict\n",
    )
    .unwrap();
    std::fs::set_permissions(
        &prompter,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    let (_daemon, _connection) = async_io::block_on(async {
        let daemon = Daemon::new(Keychain::load(home.path().to_path_buf()).await, prompter);
        let connection = zbus::connection::Builder::address(bus.address())
            .unwrap()
            .build()
            .await
            .unwrap();
        daemon.serve(&connection).await.unwrap();
        assert!(daemon.login(Secret::text("correct horse")).await);
        daemon
            .store_internal(
                "Home Wi-Fi",
                HashMap::from([("network".to_owned(), "home".to_owned())]),
                Secret::text("wifi-secret"),
            )
            .await;
        (daemon, connection)
    });
    let client = bus.connect().unwrap();
    let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = call(
        &client,
        SERVICE_PATH,
        SERVICE,
        "SearchItems",
        &(HashMap::from([("network", "home")]),),
    );
    assert!(unlocked.is_empty(), "another app's item starts locked");
    let (opened, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
        call(&client, SERVICE_PATH, SERVICE, "Unlock", &(&locked,));
    assert!(opened.is_empty());
    let proxy = zbus::blocking::Proxy::new(
        &client,
        "org.freedesktop.secrets",
        prompt,
        "org.freedesktop.Secret.Prompt",
    )
    .unwrap();
    let mut completed = proxy.receive_signal("Completed").unwrap();
    proxy.call_method("Prompt", &("")).unwrap();
    let (dismissed, _): (bool, OwnedValue) =
        completed.next().unwrap().body().deserialize().unwrap();
    assert!(!dismissed, "the right password after a wrong one allows it");
    let (_, session): (OwnedValue, OwnedObjectPath) = call(
        &client,
        SERVICE_PATH,
        SERVICE,
        "OpenSession",
        &("plain", Value::from("")),
    );
    let (_, _, value, _): WireSecret = call(
        &client,
        locked[0].as_str(),
        "org.freedesktop.Secret.Item",
        "GetSecret",
        &(&session,),
    );
    assert_eq!(value, b"wifi-secret");
}
