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

fn networks() -> Vec<(IpAddr, IpAddr)> {
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
        if let (Some(address), Some(mask)) =
            (address_of(entry.ifa_addr), address_of(entry.ifa_netmask))
        {
            found.push((address, mask));
        }
        next = entry.ifa_next;
    }
    // SAFETY: `first` came from getifaddrs and nothing refers to the list any more.
    unsafe { libc::freeifaddrs(first) };
    found
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

/// Whether `peer` is this computer or on a network one of its interfaces is on.
pub fn on_local_network(peer: IpAddr) -> bool {
    peer.is_loopback()
        || networks().into_iter().any(|(address, mask)| {
            masked(address, mask).is_some_and(|network| masked(peer, mask) == Some(network))
        })
}
