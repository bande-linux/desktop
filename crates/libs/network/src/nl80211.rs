use std::io;
use std::os::fd::OwnedFd;

use rustix::net::netlink::{self, SocketAddrNetlink};
use rustix::net::{RecvFlags, SendFlags, recv, sendto};

use crate::kernel::{
    HEADER, NLM_F_DUMP, NLM_F_REQUEST, NLMSG_DONE, NLMSG_ERROR, RECEIVE_BUFFER, aligned, attribute,
    attributes, interface_index, netlink_socket, u16_at, u32_at,
};
use crate::{
    AP_FLAGS_PRIVACY, AP_SEC_KEY_MGMT_802_1X, AP_SEC_KEY_MGMT_PSK, AP_SEC_KEY_MGMT_SAE, Security,
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
const NL80211_CMD_GET_STATION: u8 = 17;
const NL80211_CMD_GET_SCAN: u8 = 32;
const NL80211_ATTR_IFINDEX: u16 = 3;
const NL80211_ATTR_STA_INFO: u16 = 21;
const NL80211_ATTR_BSS: u16 = 47;
const NL80211_STA_INFO_SIGNAL: u16 = 7;
const NL80211_BSS_CAPABILITY: u16 = 5;
const NL80211_BSS_INFORMATION_ELEMENTS: u16 = 6;
const NL80211_BSS_SIGNAL_MBM: u16 = 7;
const NL80211_BSS_STATUS: u16 = 9;
const NL80211_BSS_BEACON_IES: u16 = 11;
const NL80211_BSS_STATUS_ASSOCIATED: u32 = 1;
const NLA_TYPE_MASK: u16 = 0x3fff;
const CAPABILITY_PRIVACY: u16 = 0x10;
const ELEMENT_SSID: u8 = 0;
const ELEMENT_RSN: u8 = 48;
const ELEMENT_VENDOR: u8 = 221;
const RSN_SUITE: [u8; 3] = [0x00, 0x0f, 0xac];
const WPA_SUITE: [u8; 3] = [0x00, 0x50, 0xf2];
const WPA_ELEMENT: [u8; 4] = [0x00, 0x50, 0xf2, 0x01];
const RSN_ENTERPRISE: [u8; 6] = [1, 3, 5, 11, 12, 13];
const RSN_PERSONAL: [u8; 3] = [2, 4, 6];
const RSN_SAE: [u8; 4] = [8, 9, 24, 25];

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

fn family(socket: &OwnedFd) -> io::Result<Option<u16>> {
    let families = match exchange(
        socket,
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
    Ok(families.iter().find_map(|body| {
        attributes(body).find_map(|(kind, data)| {
            (kind == CTRL_ATTR_FAMILY_ID)
                .then(|| u16_at(data, 0))
                .flatten()
        })
    }))
}

/// The country the Wi-Fi hardware is set to transmit for, as its two letters, which the kernel or
/// location-aware firmware takes from the networks nearby. A country a driver keeps from the
/// card's factory settings is ignored. Nothing is sent to the network to find out.
pub fn wifi_country() -> io::Result<Option<String>> {
    let socket = netlink_socket(Some(netlink::GENERIC), 0)?;
    let Some(family) = family(&socket)? else {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Association {
    pub ssid: Vec<u8>,
    pub strength: u8,
    pub security: Security,
}

fn nested(body: &[u8], wanted: u16) -> Option<&[u8]> {
    attributes(body).find_map(|(kind, data)| (kind & NLA_TYPE_MASK == wanted).then_some(data))
}

fn elements(bytes: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut rest = bytes;
    std::iter::from_fn(move || {
        let [id, length, tail @ ..] = rest else {
            return None;
        };
        let length = usize::from(*length);
        let data = tail.get(..length)?;
        rest = &tail[length..];
        Some((*id, data))
    })
}

fn key_management(body: &[u8], suite: [u8; 3]) -> Vec<u8> {
    let count_at = |offset: usize| {
        body.get(offset..offset + 2)
            .map(|count| usize::from(u16::from_le_bytes([count[0], count[1]])))
    };
    let Some(pairwise) = count_at(6) else {
        return Vec::new();
    };
    let akm_at = 8 + 4 * pairwise;
    let Some(count) = count_at(akm_at) else {
        return Vec::new();
    };
    body.get(akm_at + 2..)
        .unwrap_or_default()
        .as_chunks::<4>()
        .0
        .iter()
        .take(count)
        .filter(|[a, b, c, _]| [*a, *b, *c] == suite)
        .map(|[.., kind]| *kind)
        .collect()
}

fn security_of(capability: u16, elements_bytes: &[u8]) -> Security {
    let mut key_flags = 0;
    for (id, data) in elements(elements_bytes) {
        let (suite, body, enterprise, personal, sae): (_, _, &[u8], &[u8], &[u8]) = match id {
            ELEMENT_RSN => (RSN_SUITE, data, &RSN_ENTERPRISE, &RSN_PERSONAL, &RSN_SAE),
            ELEMENT_VENDOR if data.starts_with(&WPA_ELEMENT) => {
                (WPA_SUITE, &data[4..], &[1], &[2], &[])
            }
            _ => continue,
        };
        for akm in key_management(body, suite) {
            if enterprise.contains(&akm) {
                key_flags |= AP_SEC_KEY_MGMT_802_1X;
            } else if personal.contains(&akm) {
                key_flags |= AP_SEC_KEY_MGMT_PSK;
            } else if sae.contains(&akm) {
                key_flags |= AP_SEC_KEY_MGMT_SAE;
            }
        }
    }
    let privacy = match capability & CAPABILITY_PRIVACY {
        0 => 0,
        _ => AP_FLAGS_PRIVACY,
    };
    Security::of(privacy, 0, key_flags)
}

fn strength(dbm: i32) -> u8 {
    let below = (dbm.clamp(-100, -40) + 40).unsigned_abs();
    (100 - 100 * below / 60) as u8
}

fn associated(bss: &[u8]) -> Option<Association> {
    let (mut status, mut capability, mut signal) = (None, 0, None);
    let (mut probed, mut beacon) = (None, None);
    for (kind, data) in attributes(bss) {
        match kind & NLA_TYPE_MASK {
            NL80211_BSS_STATUS => status = u32_at(data, 0),
            NL80211_BSS_CAPABILITY => capability = u16_at(data, 0).unwrap_or_default(),
            NL80211_BSS_SIGNAL_MBM => {
                signal = data
                    .get(..4)
                    .and_then(|mbm| mbm.try_into().ok())
                    .map(|mbm| i32::from_ne_bytes(mbm) / 100);
            }
            NL80211_BSS_INFORMATION_ELEMENTS => probed = Some(data),
            NL80211_BSS_BEACON_IES => beacon = Some(data),
            _ => {}
        }
    }
    if status != Some(NL80211_BSS_STATUS_ASSOCIATED) {
        return None;
    }
    let found = probed.or(beacon)?;
    let ssid = elements(found)
        .find_map(|(id, data)| (id == ELEMENT_SSID).then(|| data.to_vec()))
        .filter(|ssid| !ssid.is_empty() && ssid.iter().any(|byte| *byte != 0))?;
    Some(Association {
        ssid,
        strength: signal.map_or(0, strength),
        security: security_of(capability, found),
    })
}

fn station_signal(body: &[u8]) -> Option<i32> {
    let info = nested(body, NL80211_ATTR_STA_INFO)?;
    nested(info, NL80211_STA_INFO_SIGNAL)
        .and_then(|signal| signal.first())
        .map(|signal| i32::from(i8::from_ne_bytes([*signal])))
}

/// The network a Wi-Fi interface is associated with, as the kernel knows it from the scan results
/// it keeps and the station's signal. Nothing is sent to the network to find out.
pub(crate) fn association(interface: &str) -> io::Result<Option<Association>> {
    let socket = netlink_socket(Some(netlink::GENERIC), 0)?;
    let Some(family) = family(&socket)? else {
        return Ok(None);
    };
    let index = attribute(
        NL80211_ATTR_IFINDEX,
        &interface_index(interface)?.to_ne_bytes(),
    );
    let scan = exchange(&socket, family, NLM_F_DUMP, NL80211_CMD_GET_SCAN, &index)?;
    let Some(mut association) = scan
        .iter()
        .find_map(|body| nested(body, NL80211_ATTR_BSS).and_then(associated))
    else {
        return Ok(None);
    };
    match exchange(&socket, family, NLM_F_DUMP, NL80211_CMD_GET_STATION, &index) {
        Ok(stations) => {
            if let Some(dbm) = stations.iter().find_map(|body| station_signal(body)) {
                association.strength = strength(dbm);
            }
        }
        Err(error) => tracing::debug!("cannot read the signal of {interface}: {error}"),
    }
    Ok(Some(association))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bss(status: u32, capability: u16, mbm: i32, ies: &[u8]) -> Vec<u8> {
        [
            attribute(NL80211_BSS_STATUS, &status.to_ne_bytes()),
            attribute(NL80211_BSS_CAPABILITY, &capability.to_ne_bytes()),
            attribute(NL80211_BSS_SIGNAL_MBM, &mbm.to_ne_bytes()),
            attribute(NL80211_BSS_INFORMATION_ELEMENTS, ies),
        ]
        .concat()
    }

    #[test]
    fn reads_the_network_a_card_is_associated_with() {
        let ssid = [&[ELEMENT_SSID, 4][..], b"Home"].concat();
        let rsn = [
            ELEMENT_RSN,
            24,
            1,
            0,
            0x00,
            0x0f,
            0xac,
            4,
            1,
            0,
            0x00,
            0x0f,
            0xac,
            4,
            2,
            0,
            0x00,
            0x0f,
            0xac,
            2,
            0x00,
            0x0f,
            0xac,
            8,
            0x0c,
            0,
        ];
        let wpa = [
            ELEMENT_VENDOR,
            22,
            0x00,
            0x50,
            0xf2,
            0x01,
            1,
            0,
            0x00,
            0x50,
            0xf2,
            2,
            1,
            0,
            0x00,
            0x50,
            0xf2,
            2,
            1,
            0,
            0x00,
            0x50,
            0xf2,
            1,
        ];
        let home = associated(&bss(1, 0x11, -5500, &[&ssid[..], &rsn].concat()))
            .expect("an associated network");
        assert_eq!(home.ssid, b"Home");
        assert_eq!(home.security, Security::Personal { sae_only: false });
        assert_eq!(home.strength, 75);
        let office = associated(&bss(1, 0x11, -9000, &[&ssid[..], &wpa].concat()))
            .expect("an associated network");
        assert_eq!(office.security, Security::Enterprise);
        assert_eq!(office.strength, 17);
        let old = associated(&bss(1, 0x11, -4000, &ssid)).expect("an associated network");
        assert_eq!((old.security, old.strength), (Security::Wep, 100));
        let cafe = associated(&bss(1, 0x01, -7000, &ssid)).expect("an associated network");
        assert_eq!(cafe.security, Security::Open);
        assert_eq!(associated(&bss(0, 0x11, -5500, &ssid)), None);
        let hidden = [ELEMENT_SSID, 3, 0, 0, 0];
        assert_eq!(associated(&bss(1, 0x11, -5500, &hidden)), None);
    }
}
