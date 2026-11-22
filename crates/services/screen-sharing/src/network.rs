use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

fn address_of(socket: *const libc::sockaddr) -> Option<IpAddr> {
    if socket.is_null() {
        return None;
    }
    // SAFETY: getifaddrs hands out sockaddrs whose sa_family says which struct they are; each
    // is read as that struct only after checking the family.
    unsafe {
        match i32::from((*socket).sa_family) {
            libc::AF_INET => {
                let address = &*socket.cast::<libc::sockaddr_in>();
                Some(IpAddr::V4(Ipv4Addr::from(u32::from_be(
                    address.sin_addr.s_addr,
                ))))
            }
            libc::AF_INET6 => {
                let address = &*socket.cast::<libc::sockaddr_in6>();
                Some(IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr)))
            }
            _ => None,
        }
    }
}

const NET: &str = "/sys/class/net";
const ARPHRD_ETHER: &str = "1";
const MOST_LINK_DEPTH: u8 = 4;
const SHORTEST_IPV4_PREFIX: u32 = 8;
const SHORTEST_IPV6_PREFIX: u32 = 64;
const TEREDO: (u128, u32) = (0x2001_0000 << 96, 32);
const SIX_TO_FOUR: (u128, u32) = (0x2002 << 112, 16);

/// What an interface is, by what the kernel says of it rather than by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Link {
    /// An Ethernet or Wi-Fi card, or a bond, team or VLAN over one.
    Hardware,
    /// A bridge with such a link among its ports.
    Bridge,
    /// A tunnel, VPN, container or virtual machine link, or anything else.
    Virtual,
}

fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.starts_with('.')
}

/// Whether `name` is an Ethernet or Wi-Fi card, or a bond, team or VLAN over one (its `lower_`
/// links), looking at most a few links down.
fn has_hardware(name: &str, depth: u8) -> bool {
    let interface = Path::new(NET).join(name);
    let ethernet = std::fs::read_to_string(interface.join("type"))
        .is_ok_and(|kind| kind.trim() == ARPHRD_ETHER);
    if !plain_name(name) || !ethernet || interface.join("tun_flags").exists() {
        return false;
    }
    if interface.join("device").exists() {
        return true;
    }
    depth < MOST_LINK_DEPTH
        && std::fs::read_dir(&interface).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .strip_prefix("lower_")
                    .is_some_and(|lower| has_hardware(lower, depth + 1))
            })
        })
}

fn link_of(name: &str) -> Link {
    if !plain_name(name) {
        return Link::Virtual;
    }
    if has_hardware(name, 0) {
        return Link::Hardware;
    }
    let ports = std::fs::read_dir(Path::new(NET).join(name).join("brif"));
    let bridges_hardware = ports.is_ok_and(|ports| {
        ports
            .flatten()
            .any(|port| has_hardware(&port.file_name().to_string_lossy(), 0))
    });
    match bridges_hardware {
        true => Link::Bridge,
        false => Link::Virtual,
    }
}

struct Interface {
    link: Link,
    flags: u32,
    address: IpAddr,
    mask: IpAddr,
}

fn prefix_length(mask: IpAddr) -> u32 {
    match mask {
        IpAddr::V4(mask) => mask.to_bits().count_ones(),
        IpAddr::V6(mask) => mask.to_bits().count_ones(),
    }
}

fn within(address: u128, (network, length): (u128, u32)) -> bool {
    address >> (128 - length) == network >> (128 - length)
}

/// Whether an interface leads to the local network: up, a card or a bridge of one, not a
/// tunnel, VPN, container or virtual machine link, with a prefix no shorter than a LAN's.
fn is_local_link(link: Link, flags: u32, address: IpAddr, mask: IpAddr) -> bool {
    let up = libc::IFF_UP as u32 | libc::IFF_RUNNING as u32;
    let virtual_link = libc::IFF_LOOPBACK as u32 | libc::IFF_POINTOPOINT as u32;
    if flags & up != up || flags & virtual_link != 0 || link == Link::Virtual {
        return false;
    }
    match address {
        IpAddr::V4(_) => prefix_length(mask) >= SHORTEST_IPV4_PREFIX,
        IpAddr::V6(address) => {
            prefix_length(mask) >= SHORTEST_IPV6_PREFIX
                && !within(address.to_bits(), TEREDO)
                && !within(address.to_bits(), SIX_TO_FOUR)
        }
    }
}

fn interfaces() -> Vec<Interface> {
    let mut first: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `first` with a list that freeifaddrs releases below.
    if unsafe { libc::getifaddrs(&mut first) } != 0 {
        tracing::warn!(
            "cannot list this computer's networks: {}",
            std::io::Error::last_os_error()
        );
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut next = first;
    while !next.is_null() {
        // SAFETY: `next` is a node of the list getifaddrs returned, not yet freed.
        let entry = unsafe { &*next };
        // SAFETY: ifa_name is a NUL-terminated string of the same list.
        let name = unsafe { std::ffi::CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .into_owned();
        if let (Some(address), Some(mask)) =
            (address_of(entry.ifa_addr), address_of(entry.ifa_netmask))
        {
            found.push(Interface {
                link: link_of(&name),
                flags: entry.ifa_flags,
                address,
                mask,
            });
        }
        next = entry.ifa_next;
    }
    // SAFETY: `first` came from getifaddrs and nothing refers to the list any more.
    unsafe { libc::freeifaddrs(first) };
    found
}

fn networks() -> Vec<(IpAddr, IpAddr)> {
    interfaces()
        .into_iter()
        .filter(|interface| {
            is_local_link(
                interface.link,
                interface.flags,
                interface.address,
                interface.mask,
            )
        })
        .map(|interface| (interface.address, interface.mask))
        .collect()
}

fn masked(address: IpAddr, mask: IpAddr) -> Option<u128> {
    match (address, mask) {
        (IpAddr::V4(address), IpAddr::V4(mask)) => {
            Some(u128::from(address.to_bits() & mask.to_bits()))
        }
        (IpAddr::V6(address), IpAddr::V6(mask)) => Some(address.to_bits() & mask.to_bits()),
        _ => None,
    }
}

/// Whether `peer` is this computer or on a network one of its real LAN interfaces is on.
pub fn on_local_network(peer: IpAddr) -> bool {
    peer.is_loopback()
        || networks().into_iter().any(|(address, mask)| {
            masked(address, mask).is_some_and(|network| masked(peer, mask) == Some(network))
        })
}

/// The addresses viewers on the local network can reach this computer at.
pub fn addresses() -> Vec<IpAddr> {
    let mut found: Vec<IpAddr> = networks()
        .into_iter()
        .map(|(address, _)| address)
        .filter(|address| match address {
            IpAddr::V4(address) => !address.is_loopback() && !address.is_link_local(),
            IpAddr::V6(_) => false,
        })
        .collect();
    found.dedup();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_lan_links_count_as_the_local_network() {
        let up = (libc::IFF_UP | libc::IFF_RUNNING | libc::IFF_BROADCAST) as u32;
        let v4 = |text: &str| IpAddr::V4(text.parse().unwrap());
        let v6 = |text: &str| IpAddr::V6(text.parse().unwrap());
        let lan = v6("ffff:ffff:ffff:ffff::");
        let class_c = v4("255.255.255.0");
        assert!(is_local_link(
            Link::Hardware,
            up,
            v4("192.168.1.20"),
            class_c
        ));
        assert!(is_local_link(Link::Bridge, up, v4("192.168.1.20"), class_c));
        assert!(is_local_link(Link::Hardware, up, v6("fe80::1"), lan));
        let down = libc::IFF_BROADCAST as u32;
        assert!(!is_local_link(
            Link::Hardware,
            down,
            v4("192.168.1.20"),
            class_c
        ));
        assert!(!is_local_link(Link::Virtual, up, v4("10.8.0.2"), class_c));
        let point_to_point = up | libc::IFF_POINTOPOINT as u32;
        assert!(!is_local_link(
            Link::Hardware,
            point_to_point,
            v4("10.0.0.2"),
            v4("255.0.0.0")
        ));
        assert!(!is_local_link(
            Link::Hardware,
            up,
            v4("10.0.0.2"),
            v4("0.0.0.0")
        ));
        assert!(!is_local_link(
            Link::Hardware,
            up,
            v6("2001:0:53aa:64c::1"),
            lan
        ));
        assert!(!is_local_link(
            Link::Hardware,
            up,
            v6("2002:c000:204::1"),
            lan
        ));
        assert!(!is_local_link(
            Link::Hardware,
            up,
            v6("2a00:1450::1"),
            v6("ffff:ffff::")
        ));
        assert_eq!(link_of("../lo"), Link::Virtual);
        assert_eq!(link_of("lo"), Link::Virtual);
    }
}
