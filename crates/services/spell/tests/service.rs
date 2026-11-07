#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::fs;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use zbus::blocking::{Connection, Proxy};

#[test]
fn a_made_up_language_needs_only_data_and_learned_words_reach_clients() {
    let Ok(address) = std::env::var("DBUS_SESSION_BUS_ADDRESS") else {
        return;
    };
    let root = std::env::temp_dir().join(format!("bande-spell-{}", std::process::id()));
    let hunspell = root.join("share/hunspell");
    let text = root.join("share/bande/text");
    fs::create_dir_all(&hunspell).unwrap();
    fs::create_dir_all(&text).unwrap();
    fs::write(hunspell.join("xq_XQ.aff"), "SET UTF-8\nTRY abdiklmnoruz\n").unwrap();
    fs::write(hunspell.join("xq_XQ.dic"), "2\nzorblak\nmindu\n").unwrap();
    fs::write(text.join("xq"), "dictionary = xq_XQ\nquote-open = «\nautocorrect = true\n").unwrap();
    let mut service = Command::new(env!("CARGO_BIN_EXE_bande-spell"))
        .env("DBUS_SESSION_BUS_ADDRESS", address)
        .env("HOME", &root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_DATA_DIRS", root.join("share"))
        .env("LANG", "xq_XQ.UTF-8")
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let connection = Connection::session().unwrap();
    let proxy = Proxy::new(
        &connection,
        bande_spell::BUS_NAME,
        bande_spell::OBJECT_PATH,
        "org.bande.Spell",
    )
    .unwrap();
    let words = vec!["zorblak".to_owned(), "zorbalk".to_owned()];
    let deadline = Instant::now() + Duration::from_secs(10);
    let verdicts: Vec<bool> = loop {
        match proxy.call("Check", &(&words,)) {
            Ok(verdicts) => break verdicts,
            Err(_) if Instant::now() < deadline => sleep(Duration::from_millis(20)),
            Err(error) => panic!("{error}"),
        }
    };
    assert_eq!(verdicts, [true, false]);
    let (rules, _): (HashMap<String, String>, HashMap<String, String>) =
        proxy.call("Rules", &()).unwrap();
    assert_eq!(rules["quote-open"], "«");
    let changes = proxy.receive_signal("Changed").unwrap();
    let _: () = proxy.call("Learn", &("zorbalk",)).unwrap();
    assert!(changes.into_iter().next().is_some());
    let verdicts: Vec<bool> = proxy.call("Check", &(&words,)).unwrap();
    assert_eq!(verdicts, [true, true]);
    service.kill().unwrap();
    service.wait().unwrap();
    fs::remove_dir_all(root).unwrap();
}
