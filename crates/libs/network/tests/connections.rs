#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::Ipv4Addr;
use std::path::Path;

use unbound_network::{ManualIpv4, ipv4_settings, parse_manual, vpn_type_of};
use zbus::zvariant::OwnedValue;

#[test]
fn a_manual_address_is_read_the_way_people_type_it() {
    assert_eq!(
        parse_manual("192.168.1.20/24", "192.168.1.1", "1.1.1.1, 9.9.9.9"),
        Some(ManualIpv4 {
            address: Ipv4Addr::new(192, 168, 1, 20),
            prefix: 24,
            router: Some(Ipv4Addr::new(192, 168, 1, 1)),
            dns: vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)],
        })
    );
    assert_eq!(
        parse_manual(" 10.0.0.5 ", "", "").map(|manual| (manual.prefix, manual.router)),
        Some((24, None))
    );
    assert_eq!(parse_manual("10.0.0.5/33", "", ""), None);
    assert_eq!(parse_manual("10.0.0", "", ""), None);
    assert_eq!(parse_manual("10.0.0.5", "router", ""), None);
}

#[test]
fn ipv4_settings_switch_between_automatic_and_manual() {
    let method = |settings: &std::collections::HashMap<String, OwnedValue>| {
        String::try_from(settings["method"].try_clone().expect("clone")).expect("text")
    };
    assert_eq!(method(&ipv4_settings(None)), "auto");
    let manual = parse_manual("192.168.1.20/24", "192.168.1.1", "1.1.1.1").expect("manual");
    let settings = ipv4_settings(Some(&manual));
    assert_eq!(method(&settings), "manual");
    assert_eq!(
        String::try_from(settings["gateway"].try_clone().expect("clone")).expect("text"),
        "192.168.1.1"
    );
    assert_eq!(
        Vec::<u32>::try_from(settings["dns"].try_clone().expect("clone")).expect("dns"),
        [u32::from_le_bytes([1, 1, 1, 1])]
    );
}

#[test]
fn vpn_files_are_recognised_by_their_extension() {
    assert_eq!(
        vpn_type_of(Path::new("/tmp/office.conf")),
        Some("wireguard")
    );
    assert_eq!(vpn_type_of(Path::new("work.ovpn")), Some("openvpn"));
    assert_eq!(vpn_type_of(Path::new("notes.txt")), None);
}
