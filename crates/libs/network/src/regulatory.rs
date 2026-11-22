use std::io;
use std::os::fd::OwnedFd;

use rustix::net::netlink::{self, SocketAddrNetlink};
use rustix::net::{RecvFlags, SendFlags, recv, sendto};

use crate::kernel::{
    HEADER, NLM_F_DUMP, NLM_F_REQUEST, NLMSG_DONE, NLMSG_ERROR, RECEIVE_BUFFER, aligned, attribute,
    attributes, netlink_socket, u16_at, u32_at,
};

const GENERIC_HEADER: usize = 4;
const GENL_ID_CTRL: u16 = 16;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;
const NL80211: &[u8] = b"nl80211\0";
const NL80211_CMD_GET_REG: u8 = 31;
const NL80211_ATTR_WIPHY: u16 = 1;
const NL80211_ATTR_REG_ALPHA2: u16 = 33;
const NL80211_ATTR_WIPHY_SELF_MANAGED_REG: u16 = 216;

fn exchange(
    socket: &OwnedFd,
    family: u16,
    flags: u16,
    command: u8,
    payload: &[u8],
) -> io::Result<Vec<Vec<u8>>> {
    let length = HEADER + GENERIC_HEADER + payload.len();
    let mut request = Vec::with_capacity(length);
    request.extend_from_slice(&(length as u32).to_ne_bytes());
    request.extend_from_slice(&family.to_ne_bytes());
    request.extend_from_slice(&(NLM_F_REQUEST | flags).to_ne_bytes());
    request.extend_from_slice(&1u32.to_ne_bytes());
    request.extend_from_slice(&0u32.to_ne_bytes());
    request.extend_from_slice(&[command, 1, 0, 0]);
    request.extend_from_slice(payload);
    sendto(
        socket,
        &request,
        SendFlags::empty(),
        &SocketAddrNetlink::new(0, 0),
    )?;
    let mut bodies = Vec::new();
    let mut buffer = vec![0; RECEIVE_BUFFER];
    loop {
        let (read, _) = recv(socket, &mut buffer[..], RecvFlags::empty())?;
        let received = &buffer[..read];
        let mut offset = 0;
        while let (Some(length), Some(kind)) =
            (u32_at(received, offset), u16_at(received, offset + 4))
        {
            let length = length as usize;
            let Some(message) = received.get(offset..offset + length) else {
                break;
            };
            match kind {
                NLMSG_DONE => return Ok(bodies),
                NLMSG_ERROR => {
                    let code = message
                        .get(HEADER..HEADER + 4)
                        .and_then(|code| Some(i32::from_ne_bytes(code.try_into().ok()?)));
                    return match code {
                        Some(code) if code < 0 => Err(io::Error::from_raw_os_error(-code)),
                        _ => Ok(bodies),
                    };
                }
                _ => {
                    if let Some(body) = message.get(HEADER + GENERIC_HEADER..) {
                        bodies.push(body.to_vec());
                    }
                }
            }
            offset += aligned(length.max(HEADER));
        }
        if read == 0 || flags & NLM_F_DUMP == 0 {
            return Ok(bodies);
        }
    }
}

fn country(code: &[u8]) -> Option<String> {
    let code = code.strip_suffix(b"\0").unwrap_or(code);
    (code.len() == 2 && code.iter().all(u8::is_ascii_uppercase))
        .then(|| String::from_utf8_lossy(code).into_owned())
}

/// The country the Wi-Fi hardware is set to transmit for, as its two letters, which the kernel or
/// location-aware firmware takes from the networks nearby. A country a driver keeps from the
/// card's factory settings is ignored. Nothing is sent to the network to find out.
pub fn wifi_country() -> io::Result<Option<String>> {
    let socket = netlink_socket(Some(netlink::GENERIC), 0)?;
    let families = match exchange(
        &socket,
        GENL_ID_CTRL,
        0,
        CTRL_CMD_GETFAMILY,
        &attribute(CTRL_ATTR_FAMILY_NAME, NL80211),
    ) {
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::NOENT.raw_os_error()) => {
            return Ok(None);
        }
        families => families?,
    };
    let Some(family) = families.iter().find_map(|body| {
        attributes(body).find_map(|(kind, data)| {
            (kind == CTRL_ATTR_FAMILY_ID)
                .then(|| u16_at(data, 0))
                .flatten()
        })
    }) else {
        return Ok(None);
    };
    let domains = exchange(&socket, family, NLM_F_DUMP, NL80211_CMD_GET_REG, &[])?;
    let mut global = None;
    for body in &domains {
        let mut wiphy = false;
        let mut self_managed = false;
        let mut code = None;
        for (kind, data) in attributes(body) {
            match kind {
                NL80211_ATTR_WIPHY => wiphy = true,
                NL80211_ATTR_WIPHY_SELF_MANAGED_REG => self_managed = true,
                NL80211_ATTR_REG_ALPHA2 => code = country(data),
                _ => {}
            }
        }
        match (wiphy, self_managed, code) {
            (true, true, Some(code)) => return Ok(Some(code)),
            (false, _, Some(code)) => global = Some(code),
            _ => {}
        }
    }
    Ok(global)
}
