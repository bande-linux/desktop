use std::os::fd::AsFd;
use std::time::Duration;
use std::{fmt, io};

use smithay::backend::drm::DrmDevice;
use smithay::reexports::drm::control::atomic::AtomicModeReq;
use smithay::reexports::drm::control::{AtomicCommitFlags, Device as _, crtc, property};

use crate::night::{NEUTRAL, matrix, ramp};

const RAMP_SLACK: u16 = 1;
const ENTRY: usize = 8;
const RETRIES: u32 = 5;
const FIRST_RETRY: Duration = Duration::from_millis(250);

/// How a CRTC is tinted: with a table of its own size, with a legacy ramp where that is all there
/// is, with a matrix, or not at all.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tint {
    Table(Table),
    Ramp(Ramp),
    Matrix(property::Handle),
    Fixed,
}

impl fmt::Display for Tint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Table(table) => write!(f, "a table of {} entries", table.size),
            Self::Ramp(ramp) => write!(f, "a legacy ramp of {} entries", ramp.size),
            Self::Matrix(_) => f.write_str("a colour matrix"),
            Self::Fixed => f.write_str("nothing"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    gamma: property::Handle,
    size: usize,
    before: Vec<property::Handle>,
}

/// A legacy ramp. Where it stands in for a table the kernel keeps it in that table, and only the
/// table's property says whether one is shown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Ramp {
    size: usize,
    within: Option<Table>,
}

/// When a tint that failed is tried again by itself: a few times, each after twice the wait.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct TintRetry {
    tries: u32,
    due: Option<Duration>,
}

impl TintRetry {
    pub(crate) fn failed(&mut self, now: Duration) {
        if self.due.is_none() && self.tries < RETRIES {
            self.due = Some(now + FIRST_RETRY * 2_u32.pow(self.tries));
        }
    }

    pub(crate) fn is_due(&mut self, now: Duration) -> bool {
        let due = self.due.take_if(|due| *due <= now).is_some();
        self.tries += u32::from(due);
        due
    }

    pub(crate) fn wait(self, now: Duration) -> Option<Duration> {
        self.due.map(|due| due.saturating_sub(now))
    }
}

impl Tint {
    pub(crate) fn of(device: &DrmDevice, crtc: crtc::Handle) -> Self {
        let named = properties(device, crtc);
        let find = |name: &str| named.iter().find(|(known, ..)| known == name);
        let table = find("GAMMA_LUT")
            .zip(find("GAMMA_LUT_SIZE"))
            .filter(|_| device.is_atomic())
            .map(|((_, gamma, _), (_, _, size))| (*gamma, *size as usize))
            .filter(|(_, size)| *size > 0);
        if let Some((gamma, size)) = table {
            let before = ["DEGAMMA_LUT", "CTM"]
                .into_iter()
                .filter_map(find)
                .map(|(_, property, _)| *property)
                .collect();
            return Self::Table(Table {
                gamma,
                size,
                before,
            });
        }
        let length = device
            .get_crtc(crtc)
            .map_or(0, |info| info.gamma_length() as usize);
        match find("CTM") {
            _ if length > 0 => Self::Ramp(Ramp {
                size: length,
                within: None,
            }),
            Some((_, property, _)) => Self::Matrix(*property),
            None => Self::Fixed,
        }
    }

    pub(crate) fn instead(&self, device: &DrmDevice, crtc: crtc::Handle) -> Option<Self> {
        if let Self::Table(table)
        | Self::Ramp(Ramp {
            within: Some(table),
            ..
        }) = self
            && let Err(error) = table.clear(device, crtc)
        {
            tracing::warn!("a colour table stays: {error}");
        }
        let length = device
            .get_crtc(crtc)
            .map_or(0, |info| info.gamma_length() as usize);
        match self {
            Self::Table(table) if length > 0 => Some(Self::Ramp(Ramp {
                size: length,
                within: Some(table.clone()),
            })),
            Self::Table(_) | Self::Ramp(_) | Self::Matrix(_) => Some(Self::Fixed),
            Self::Fixed => None,
        }
    }

    /// Shows `kelvin`, or takes every tint away when it is neutral, doing nothing when there is
    /// none to take away.
    pub(crate) fn show(
        &self,
        device: &DrmDevice,
        crtc: crtc::Handle,
        kelvin: f64,
    ) -> io::Result<()> {
        let neutral = kelvin == NEUTRAL;
        match self {
            Self::Table(table) => match neutral {
                true => table.clear(device, crtc),
                false => table.set(device, crtc, kelvin),
            },
            Self::Ramp(Ramp {
                within: Some(table),
                ..
            }) if neutral => table.clear(device, crtc),
            Self::Ramp(Ramp { size, .. }) => {
                if neutral && shows_linear(device, crtc, *size)? {
                    return Ok(());
                }
                let [red, green, blue] = ramp(*size, kelvin);
                device.set_gamma(crtc, &red, &green, &blue)
            }
            Self::Matrix(property) => match neutral {
                true if value_of(device, crtc, *property) == 0 => Ok(()),
                true => device.set_property(crtc, *property, 0),
                false => {
                    let blob = device.create_property_blob(&matrix(kelvin))?;
                    let set = device.set_property(crtc, *property, blob.into());
                    device.destroy_property_blob(blob.into())?;
                    set
                }
            },
            Self::Fixed => Ok(()),
        }
    }
}

impl Table {
    fn clear(&self, device: &DrmDevice, crtc: crtc::Handle) -> io::Result<()> {
        let mut request = AtomicModeReq::new();
        let mut left = false;
        for property in self.before.iter().chain([&self.gamma]) {
            if value_of(device, crtc, *property) != 0 {
                request.add_property(crtc, *property, property::Value::Blob(0));
                left = true;
            }
        }
        match left {
            true => device.atomic_commit(AtomicCommitFlags::empty(), request),
            false => Ok(()),
        }
    }

    fn set(&self, device: &DrmDevice, crtc: crtc::Handle, kelvin: f64) -> io::Result<()> {
        let mut entries = entries(&ramp(self.size, kelvin));
        let blob = drm_ffi::mode::create_property_blob(device.as_fd(), &mut entries)?.blob_id;
        let mut request = AtomicModeReq::new();
        for property in &self.before {
            if value_of(device, crtc, *property) != 0 {
                request.add_property(crtc, *property, property::Value::Blob(0));
            }
        }
        request.add_property(crtc, self.gamma, property::Value::Blob(u64::from(blob)));
        let set = device.atomic_commit(AtomicCommitFlags::empty(), request);
        device.destroy_property_blob(u64::from(blob))?;
        set
    }
}

fn properties(device: &DrmDevice, crtc: crtc::Handle) -> Vec<(String, property::Handle, u64)> {
    let Ok(properties) = device.get_properties(crtc) else {
        return Vec::new();
    };
    properties
        .iter()
        .filter_map(|(property, value)| {
            let info = device.get_property(*property).ok()?;
            let name = info.name().to_string_lossy().into_owned();
            Some((name, *property, *value))
        })
        .collect()
}

fn value_of(device: &DrmDevice, crtc: crtc::Handle, wanted: property::Handle) -> u64 {
    device.get_properties(crtc).map_or(0, |properties| {
        properties
            .iter()
            .find(|(property, _)| **property == wanted)
            .map_or(0, |(_, value)| *value)
    })
}

fn shows_linear(device: &DrmDevice, crtc: crtc::Handle, size: usize) -> io::Result<bool> {
    let mut shown = [vec![0; size], vec![0; size], vec![0; size]];
    let [red, green, blue] = &mut shown;
    device.get_gamma(crtc, red, green, blue)?;
    Ok(shown.iter().all(|channel| is_linear(channel)))
}

fn is_linear(channel: &[u16]) -> bool {
    let last = channel.len().saturating_sub(1).max(1) as f64;
    channel.iter().enumerate().all(|(step, value)| {
        let wanted = (step as f64 / last * f64::from(u16::MAX)).round() as u16;
        let coarse = u16::from((value >> 8) as u8) << 8;
        value.abs_diff(wanted) <= RAMP_SLACK || coarse == *value && wanted.abs_diff(*value) < 256
    })
}

fn entries([red, green, blue]: &[Vec<u16>; 3]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(red.len() * ENTRY);
    for ((red, green), blue) in red.iter().zip(green).zip(blue) {
        for value in [*red, *green, *blue, 0] {
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_tint_is_tried_again_a_few_times_with_growing_waits_and_then_wakes_nothing() {
        let mut retry = TintRetry::default();
        let mut now = Duration::ZERO;
        assert_eq!(retry.wait(now), None);
        retry.failed(now);
        let mut waits = Vec::new();
        while let Some(wait) = retry.wait(now) {
            assert!(!retry.is_due(now), "not before its time");
            retry.failed(now);
            assert_eq!(retry.wait(now), Some(wait), "a flip puts nothing off");
            waits.push(wait.as_millis());
            now += wait;
            assert!(retry.is_due(now));
            retry.failed(now);
        }
        assert_eq!(waits, [250, 500, 1000, 2000, 4000]);
        retry.failed(now);
        assert_eq!(retry.wait(now), None, "flips alone do not wake it again");
    }

    #[test]
    fn only_a_straight_ramp_counts_as_none_and_a_table_fills_every_entry() {
        for size in [2, 256, 1024, 4096] {
            let [red, ..] = ramp(size, NEUTRAL);
            assert!(is_linear(&red), "{size} entries");
            let coarse: Vec<u16> = red.iter().map(|value| value & 0xff00).collect();
            assert!(is_linear(&coarse), "{size} entries of 8 bits");
            let warm = ramp(size, 3000.0);
            assert!(!is_linear(&warm[2]), "{size} warm entries");
            let bytes = entries(&warm);
            assert_eq!(bytes.len(), size * ENTRY);
            let last = &bytes[(size - 1) * ENTRY..];
            let value = |at: usize| u16::from_ne_bytes([last[at], last[at + 1]]);
            assert_eq!(value(0), u16::MAX, "red is kept whole");
            assert!(value(2) > value(4) && value(4) > 0, "green above blue");
            assert_eq!(value(6), 0);
        }
        assert!(!is_linear(&[0; 256]), "a black ramp");
    }
}
