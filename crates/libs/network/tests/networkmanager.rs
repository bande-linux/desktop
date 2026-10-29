#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use unbound_network::{Error, JoinOutcome, NetworkManager, Security, Wired};
use unbound_test_bus::SessionBus;
use unbound_test_bus::networkmanager::FakeNetworkManager;

struct Fixture {
    fake: FakeNetworkManager,
    client: NetworkManager,
    _bus: SessionBus,
}

fn start() -> Fixture {
    let bus = SessionBus::start().expect("dbus-daemon");
    let fake = FakeNetworkManager::start(&bus).expect("fake NetworkManager");
    let client = NetworkManager::new(&bus.connect().expect("client connection"));
    Fixture {
        fake,
        client,
        _bus: bus,
    }
}

impl Fixture {
    fn network(&self, name: &str) -> unbound_network::Network {
        self.client
            .networks()
            .expect("networks")
            .into_iter()
            .find(|network| network.name() == name)
            .expect("network listed")
    }
}

#[test]
fn turning_wifi_off_and_on_writes_wireless_enabled() {
    let fixture = start();
    assert!(fixture.client.wifi_enabled().expect("read"));
    fixture.client.set_wifi_enabled(false).expect("off");
    assert!(!fixture.fake.wireless_enabled().expect("manager"));
    fixture.client.set_wifi_enabled(true).expect("on");
    assert!(fixture.fake.wireless_enabled().expect("manager"));
}

#[test]
fn network_list_shows_one_row_per_name_current_then_known_then_strongest() {
    let fixture = start();
    let networks = fixture.client.networks().expect("networks");
    let rows: Vec<(String, u8, bool, bool)> = networks
        .iter()
        .map(|network| {
            (
                network.name(),
                network.strength,
                network.current,
                network.known,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("Home".to_owned(), 70, true, false),
            ("Studio".to_owned(), 30, false, true),
            ("Cafe".to_owned(), 90, false, false),
            ("Office".to_owned(), 80, false, false),
            ("Modern".to_owned(), 60, false, false),
        ]
    );
    let security: Vec<Security> = networks.iter().map(|network| network.security).collect();
    assert_eq!(
        security,
        [
            Security::Personal { sae_only: false },
            Security::Personal { sae_only: false },
            Security::Open,
            Security::Enterprise,
            Security::Personal { sae_only: true },
        ]
    );
}

#[test]
fn opening_the_list_asks_for_a_scan() {
    let fixture = start();
    fixture.client.scan().expect("scan");
    assert_eq!(fixture.fake.scans(), 1);
}

#[test]
fn joining_an_open_network_needs_no_password_and_remembers_it() {
    let fixture = start();
    let cafe = fixture.network("Cafe");
    assert_eq!(
        fixture.client.join(&cafe, None).expect("join"),
        JoinOutcome::Joined
    );
    assert!(fixture.fake.saved_ssids().contains(&b"Cafe".to_vec()));
    assert!(fixture.network("Cafe").known);
}

#[test]
fn a_secured_unknown_network_asks_for_a_password_first() {
    let fixture = start();
    let home = fixture.network("Home");
    assert!(matches!(
        fixture.client.join(&home, None),
        Err(Error::PasswordRequired)
    ));
    assert_eq!(fixture.fake.saved_ssids(), [b"Studio".to_vec()]);
}

#[test]
fn a_wrong_password_is_reported_and_not_remembered() {
    let fixture = start();
    let home = fixture.network("Home");
    assert_eq!(
        fixture.client.join(&home, Some("wrong")).expect("join"),
        JoinOutcome::WrongPassword
    );
    assert_eq!(fixture.fake.saved_ssids(), [b"Studio".to_vec()]);
    assert_eq!(
        fixture
            .client
            .join(&home, Some(FakeNetworkManager::HOME_PASSWORD))
            .expect("join"),
        JoinOutcome::Joined
    );
    assert!(fixture.fake.saved_ssids().contains(&b"Home".to_vec()));
}

#[test]
fn a_known_network_rejoins_with_its_saved_password_or_a_new_one() {
    let fixture = start();
    let studio = fixture.network("Studio");
    assert_eq!(
        fixture.client.join(&studio, None).expect("join"),
        JoinOutcome::WrongPassword
    );
    assert_eq!(
        fixture
            .client
            .join(&studio, Some(FakeNetworkManager::STUDIO_PASSWORD))
            .expect("join"),
        JoinOutcome::Joined
    );
    assert_eq!(fixture.fake.saved_ssids(), [b"Studio".to_vec()]);
}

#[test]
fn enterprise_networks_are_not_joined_with_a_personal_password() {
    let fixture = start();
    let office = fixture.network("Office");
    assert!(matches!(
        fixture.client.join(&office, Some("secret")),
        Err(Error::Unsupported(Security::Enterprise))
    ));
}

#[test]
fn wired_status_reports_cable_and_connection() {
    let fixture = start();
    assert_eq!(
        fixture.client.wired().expect("wired"),
        Some(Wired {
            cable: true,
            connected: false
        })
    );
}
