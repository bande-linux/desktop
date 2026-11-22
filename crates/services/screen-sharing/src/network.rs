use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

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

const VIRTUAL: [&str; 24] = [
    "tun",
    "tap",
    "wg",
    "ppp",
    "docker",
    "br-",
    "virbr",
    "veth",
    "vnet",
    "lxc",
    "lxdbr",
    "incusbr",
    "zt",
    "tailscale",
    "nordlynx",
    "vmnet",
    "vboxnet",
    "cni",
    "flannel",
    "podman",
    "sit",
    "gre",
    "ip6tnl",
    "teredo",
];
const SHORTEST_IPV4_PREFIX: u32 = 8;
const SHORTEST_IPV6_PREFIX: u32 = 64;
const TEREDO: (u128, u32) = (0x2001_0000 << 96, 32);
const SIX_TO_FOUR: (u128, u32) = (0x2002 << 112, 16);

struct Interface {
    name: String,
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

/// Whether an interface leads to the local network: up, a real link and not a tunnel, VPN,
/// container or virtual machine bridge, with a prefix no shorter than a LAN's.
fn is_local_link(name: &str, flags: u32, address: IpAddr, mask: IpAddr) -> bool {
    let up = libc::IFF_UP as u32 | libc::IFF_RUNNING as u32;
    let virtual_link = libc::IFF_LOOPBACK as u32 | libc::IFF_POINTOPOINT as u32;
    if flags & up != up || flags & virtual_link != 0 {
        return false;
    }
    if VIRTUAL.iter().any(|prefix| name.starts_with(prefix)) {
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
                name,
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
                &interface.name,
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
        assert!(is_local_link(
            "wlp2s0",
            up,
            v4("192.168.1.20"),
            v4("255.255.255.0")
        ));
        assert!(is_local_link("enp3s0", up, v6("fe80::1"), lan));
        assert!(!is_local_link(
            "enp3s0",
            libc::IFF_BROADCAST as u32,
            v4("192.168.1.20"),
            v4("255.255.255.0")
        ));
        assert!(!is_local_link(
            "wg0",
            up,
            v4("10.8.0.2"),
            v4("255.255.255.0")
        ));
        assert!(!is_local_link(
            "docker0",
            up,
            v4("172.17.0.1"),
            v4("255.255.0.0")
        ));
        assert!(!is_local_link(
            "eth0",
            up | libc::IFF_POINTOPOINT as u32,
            v4("10.0.0.2"),
            v4("255.0.0.0")
        ));
        assert!(!is_local_link("eth0", up, v4("10.0.0.2"), v4("0.0.0.0")));
        assert!(!is_local_link("eth0", up, v6("2001:0:53aa:64c::1"), lan));
        assert!(!is_local_link("eth0", up, v6("2002:c000:204::1"), lan));
        assert!(!is_local_link(
            "eth0",
            up,
            v6("2a00:1450::1"),
            v6("ffff:ffff::")
        ));
    }
}
