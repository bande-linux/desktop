use std::fs;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::fd::OwnedFd;
use std::sync::mpsc::Sender;

use rustix::net::netlink::SocketAddrNetlink;
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketFlags, SocketType, bind, recv, sendto, socket_with,
};

use crate::Addressing;

const HEADER: usize = 16;
const ADDRESS_HEADER: usize = 8;
const ATTRIBUTE_HEADER: usize = 4;
const RTM_NEWADDR: u16 = 20;
const RTM_GETADDR: u16 = 22;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLM_F_REQUEST: u16 = 0x1;
const NLM_F_DUMP: u16 = 0x300;
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;
const RT_SCOPE_UNIVERSE: u8 = 0;
const RTMGRP_LINK: u32 = 0x1;
const RTMGRP_IPV4_IFADDR: u32 = 0x10;
const RTMGRP_IPV4_ROUTE: u32 = 0x40;
const RTMGRP_IPV6_IFADDR: u32 = 0x100;
const RTMGRP_IPV6_ROUTE: u32 = 0x400;
const RTF_UP: u32 = 0x1;
const RTF_REJECT: u32 = 0x200;
const LOOPBACK: &str = "lo";
const ROUTES: &str = "/proc/net/route";
const IPV6_ROUTES: &str = "/proc/net/ipv6_route";
const RESOLVER: &str = "/etc/resolv.conf";
const RECEIVE_BUFFER: usize = 32 * 1024;

fn route_socket(groups: u32) -> io::Result<OwnedFd> {
    let socket = socket_with(
        AddressFamily::NETLINK,
        SocketType::RAW,
        SocketFlags::CLOEXEC,
        None,
    )?;
    bind(&socket, &SocketAddrNetlink::new(0, groups))?;
    Ok(socket)
}

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn aligned(length: usize) -> usize {
    (length + 3) & !3
}

fn address_of(family: u8, prefix: u8, attributes: &[u8]) -> Option<String> {
    let mut offset = 0;
    let (mut local, mut address) = (None, None);
    while let Some(length) = u16_at(attributes, offset).map(usize::from) {
        if length < ATTRIBUTE_HEADER {
            return None;
        }
        let kind = u16_at(attributes, offset + 2)?;
        let data = attributes.get(offset + ATTRIBUTE_HEADER..offset + length)?;
        let text = match (family, data.len()) {
            (AF_INET, 4) => Ipv4Addr::from(<[u8; 4]>::try_from(data).ok()?).to_string(),
            (AF_INET6, 16) => Ipv6Addr::from(<[u8; 16]>::try_from(data).ok()?).to_string(),
            _ => String::new(),
        };
        match kind {
            IFA_LOCAL if !text.is_empty() => local = Some(text),
            IFA_ADDRESS if !text.is_empty() => address = Some(text),
            _ => {}
        }
        offset += aligned(length);
    }
    local
        .or(address)
        .map(|address| format!("{address}/{prefix}"))
}

fn read_addresses(messages: &[u8], index: u32, found: &mut Vec<String>) -> bool {
    let mut offset = 0;
    while let (Some(length), Some(kind)) = (u32_at(messages, offset), u16_at(messages, offset + 4))
    {
        let length = length as usize;
        let Some(message) = messages.get(offset..offset + length) else {
            return true;
        };
        if length < HEADER || matches!(kind, NLMSG_DONE | NLMSG_ERROR) {
            return true;
        }
        if kind == RTM_NEWADDR
            && let Some(body) = message.get(HEADER..)
            && body.len() >= ADDRESS_HEADER
            && u32_at(body, 4) == Some(index)
            && body[3] == RT_SCOPE_UNIVERSE
            && let Some(address) = address_of(body[0], body[1], &body[ADDRESS_HEADER..])
        {
            found.push(address);
        }
        offset += aligned(length);
    }
    false
}

fn addresses(interface: &str) -> io::Result<Vec<String>> {
    let index: u32 = fs::read_to_string(format!("/sys/class/net/{interface}/ifindex"))?
        .trim()
        .parse()
        .map_err(io::Error::other)?;
    let socket = route_socket(0)?;
    let mut request = Vec::with_capacity(HEADER + ADDRESS_HEADER);
    request.extend_from_slice(&((HEADER + ADDRESS_HEADER) as u32).to_ne_bytes());
    request.extend_from_slice(&RTM_GETADDR.to_ne_bytes());
    request.extend_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
    request.extend_from_slice(&1u32.to_ne_bytes());
    request.extend_from_slice(&0u32.to_ne_bytes());
    request.extend_from_slice(&[0; ADDRESS_HEADER]);
    sendto(
        &socket,
        &request,
        SendFlags::empty(),
        &SocketAddrNetlink::new(0, 0),
    )?;
    let mut found = Vec::new();
    let mut buffer = vec![0; RECEIVE_BUFFER];
    loop {
        let (read, _) = recv(&socket, &mut buffer[..], RecvFlags::empty())?;
        if read == 0 || read_addresses(&buffer[..read], index, &mut found) {
            return Ok(found);
        }
    }
}

fn default_routes(routes: &str, ipv6_routes: &str) -> Vec<(String, Option<Ipv4Addr>)> {
    let ipv4 = routes.lines().skip(1).filter_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (interface, destination, gateway, flags, mask) = (
            fields.first()?,
            fields.get(1)?,
            fields.get(2)?,
            fields.get(3)?,
            fields.get(7)?,
        );
        let flags = u32::from_str_radix(flags, 16).ok()?;
        let default = *destination == "00000000" && *mask == "00000000";
        let router = u32::from_str_radix(gateway, 16)
            .ok()
            .filter(|gateway| *gateway != 0)
            .map(|gateway| Ipv4Addr::from(gateway.to_ne_bytes()));
        (default && flags & RTF_UP != 0).then(|| ((*interface).to_owned(), router))
    });
    let ipv6 = ipv6_routes.lines().filter_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let flags = u32::from_str_radix(fields.get(8)?, 16).ok()?;
        let interface = *fields.get(9)?;
        let default = fields.first()?.bytes().all(|byte| byte == b'0') && fields.get(1)? == &"00";
        (default && flags & RTF_UP != 0 && flags & RTF_REJECT == 0 && interface != LOOPBACK)
            .then(|| (interface.to_owned(), None))
    });
    ipv4.chain(ipv6).collect()
}

fn read_default_routes() -> Vec<(String, Option<Ipv4Addr>)> {
    let read = |path| {
        fs::read_to_string(path)
            .inspect_err(|error| tracing::debug!("cannot read {path}: {error}"))
            .unwrap_or_default()
    };
    default_routes(&read(ROUTES), &read(IPV6_ROUTES))
}

pub(crate) fn routed_interfaces() -> Vec<String> {
    read_default_routes()
        .into_iter()
        .map(|(interface, _)| interface)
        .collect()
}

pub(crate) fn addressing(interface: &str) -> Addressing {
    let addresses = addresses(interface)
        .inspect_err(|error| tracing::debug!("cannot read the addresses of {interface}: {error}"))
        .unwrap_or_default();
    let routes = read_default_routes();
    let routed = routes.iter().any(|(routed, _)| routed == interface);
    Addressing {
        address: addresses.first().cloned(),
        router: routes
            .into_iter()
            .find_map(|(routed, router)| (routed == interface).then_some(router).flatten())
            .map(|router| router.to_string()),
        dns: match routed {
            true => fs::read_to_string(RESOLVER)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| line.trim().strip_prefix("nameserver"))
                .map(|server| server.trim().to_owned())
                .filter(|server| !server.is_empty())
                .collect(),
            false => Vec::new(),
        },
    }
}

pub(crate) fn follow_routes(changed: Sender<()>) -> io::Result<()> {
    let socket = route_socket(
        RTMGRP_LINK
            | RTMGRP_IPV4_IFADDR
            | RTMGRP_IPV4_ROUTE
            | RTMGRP_IPV6_IFADDR
            | RTMGRP_IPV6_ROUTE,
    )?;
    let mut buffer = vec![0; RECEIVE_BUFFER];
    loop {
        recv(&socket, &mut buffer[..], RecvFlags::empty())?;
        if changed.send(()).is_err() {
            return Ok(());
        }
    }
}
