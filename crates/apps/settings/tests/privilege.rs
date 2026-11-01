#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;

use bande_settings::{Privilege, refused};
use bande_test_bus::SessionBus;
use zbus::zvariant::OwnedValue;

struct Authority;

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl Authority {
    fn check_authorization(
        &self,
        _subject: (String, HashMap<String, OwnedValue>),
        action_id: String,
        _details: HashMap<String, String>,
        _flags: u32,
        _cancellation_id: String,
    ) -> (bool, bool, HashMap<String, String>) {
        match action_id.as_str() {
            "org.freedesktop.timedate1.set-timezone" => (true, false, HashMap::new()),
            "org.freedesktop.locale1.set-locale" => (false, true, HashMap::new()),
            _ => (false, false, HashMap::new()),
        }
    }
}

struct PartialAuthority;

#[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
impl PartialAuthority {
    fn check_authorization(
        &self,
        _subject: (String, HashMap<String, OwnedValue>),
        action_id: String,
        _details: HashMap<String, String>,
        _flags: u32,
        _cancellation_id: String,
    ) -> zbus::fdo::Result<(bool, bool, HashMap<String, String>)> {
        match action_id.as_str() {
            "org.freedesktop.timedate1.set-timezone" => Ok((true, false, HashMap::new())),
            "org.freedesktop.locale1.set-locale" => Ok((false, true, HashMap::new())),
            "org.freedesktop.hostname1.set-static-hostname" => Err(zbus::fdo::Error::Failed(
                format!("Action {action_id} is not registered"),
            )),
            _ => Ok((true, false, HashMap::new())),
        }
    }
}

fn refused_by(authority: impl zbus::object_server::Interface) -> Vec<Privilege> {
    let bus = SessionBus::start().expect("bus");
    let polkit = bus.connect().expect("polkit connection");
    polkit
        .object_server()
        .at("/org/freedesktop/PolicyKit1/Authority", authority)
        .expect("authority");
    polkit
        .request_name("org.freedesktop.PolicyKit1")
        .expect("polkit name");
    let settings = bus.connect().expect("settings connection");
    refused(&settings).expect("refused")
}

#[test]
fn only_actions_without_a_password_challenge_are_refused() {
    assert_eq!(
        refused_by(Authority),
        vec![Privilege::Hostname, Privilege::Keyboard]
    );
}

#[test]
fn an_unregistered_action_counts_as_refused_without_failing_the_others() {
    assert_eq!(refused_by(PartialAuthority), vec![Privilege::Hostname]);
}
