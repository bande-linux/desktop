#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bande_config::{ConfigService, ConfigStore};
use bande_launcher::Offers;
use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

const PATIENCE: Duration = Duration::from_secs(20);
const PATH: &str = "/org/freedesktop/Notifications";
const INTERFACE: &str = "org.freedesktop.Notifications";
const SHOWN: u32 = 7;

struct Notices(Mutex<Sender<(String, Vec<String>)>>);

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notices {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        _app: String,
        _replaces: u32,
        _icon: String,
        summary: String,
        _body: String,
        actions: Vec<String>,
        _hints: HashMap<String, OwnedValue>,
        _timeout: i32,
    ) -> u32 {
        self.0.lock().unwrap().send((summary, actions)).unwrap();
        SHOWN
    }
}

fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let until = Instant::now() + PATIENCE;
    while !done() {
        assert!(Instant::now() < until, "waited in vain for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn notices(connection: &Connection) -> Receiver<(String, Vec<String>)> {
    let (sender, receiver) = channel();
    connection
        .object_server()
        .at(PATH, Notices(Mutex::new(sender)))
        .unwrap();
    connection.request_name(INTERFACE).unwrap();
    receiver
}

#[test]
fn dark_in_search_darkens_the_desktop_and_the_notice_undoes_it() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("config.kdl");
    let stored = || std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::write(&path, "appearance \"light\"\n").unwrap();
    let config = Connection::session().unwrap();
    let _service =
        ConfigService::start(&config, &path, Arc::new(ConfigStore::default()), |_| {}).unwrap();
    let manifests = folder.path().join("intents");
    let owner = bande_settings_intents::owner();
    owner.write_manifest(&manifests).unwrap();
    let (name, settings) = (owner.bus(), Connection::session().unwrap());
    let _serving = owner.serve(&settings).unwrap();
    settings.request_name(name).unwrap();
    let notifier = Connection::session().unwrap();
    let shown = notices(&notifier);

    let mut offers = Offers::spawn(Path::new(&manifests), Connection::session).unwrap();
    offers.ask(1, "dark".to_owned());
    let mut offered = Vec::new();
    eventually("what can be done", || {
        offers.take().map(|(_, found)| offered = found).is_some()
    });
    let titles: Vec<&str> = offered.iter().map(|offer| offer.title.as_str()).collect();
    assert_eq!(titles, ["Appearance: Dark"]);

    offers.perform(offered.remove(0));
    let (summary, actions) = shown.recv_timeout(PATIENCE).unwrap();
    assert!(summary.contains("Dark"), "{summary}");
    assert!(stored().contains("appearance \"dark\""), "{}", stored());
    assert_eq!(actions[0], "undo");

    notifier
        .emit_signal(
            None::<&str>,
            PATH,
            INTERFACE,
            "ActionInvoked",
            &(SHOWN, "undo"),
        )
        .unwrap();
    eventually("the undo", || !stored().contains("\"dark\""));

    offers.ask(2, "text size 150".to_owned());
    eventually("what can be done", || {
        offers.take().map(|(_, found)| offered = found).is_some()
    });
    assert_eq!(offered[0].title, "Text size: 150");
}
