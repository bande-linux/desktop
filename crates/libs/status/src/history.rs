use zbus::blocking::Connection;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

const BATTERY_TYPE: u32 = 2;
const CHARGING: u32 = 1;
const FULL: u32 = 4;
const PENDING_CHARGE: u32 = 5;

#[zbus::proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower",
    gen_async = false,
    blocking_name = "UPower"
)]
trait UPowerManager {
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[zbus::proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower",
    gen_async = false,
    blocking_name = "PowerSource"
)]
trait PowerSourceDevice {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;

    fn get_history(
        &self,
        kind: &str,
        timespan: u32,
        resolution: u32,
    ) -> zbus::Result<Vec<(u32, f64, u32)>>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChargeSample {
    pub at: u32,
    pub percent: f64,
    pub plugged_in: bool,
}

pub fn charge_samples(records: Vec<(u32, f64, u32)>) -> Vec<ChargeSample> {
    let mut samples: Vec<ChargeSample> = records
        .into_iter()
        .filter(|(at, percent, _)| *at > 0 && *percent > 0.0)
        .map(|(at, percent, state)| ChargeSample {
            at,
            percent,
            plugged_in: matches!(state, CHARGING | FULL | PENDING_CHARGE),
        })
        .collect();
    samples.sort_by_key(|sample| sample.at);
    samples
}

pub fn charge_history(
    system: &Connection,
    seconds: u32,
    points: u32,
) -> zbus::Result<Vec<ChargeSample>> {
    for path in UPower::new(system)?.enumerate_devices()? {
        let device = PowerSource::builder(system)
            .path(path)?
            .cache_properties(CacheProperties::No)
            .build()?;
        if device.kind()? == BATTERY_TYPE && device.power_supply().unwrap_or(false) {
            return Ok(charge_samples(
                device.get_history("charge", seconds, points)?,
            ));
        }
    }
    Ok(Vec::new())
}

pub fn time_on_battery(history: &[ChargeSample], now: u32) -> Option<u32> {
    let last_plugged = history.iter().rposition(|sample| sample.plugged_in)?;
    let unplugged = history.get(last_plugged + 1)?;
    Some(now.saturating_sub(unplugged.at))
}

pub fn charging_spans(history: &[ChargeSample], now: u32) -> Vec<(u32, u32)> {
    let mut spans: Vec<(u32, u32)> = Vec::new();
    let mut since: Option<u32> = None;
    for sample in history {
        match (sample.plugged_in, since) {
            (true, None) => since = Some(sample.at),
            (false, Some(start)) => {
                spans.push((start, sample.at));
                since = None;
            }
            _ => {}
        }
    }
    spans.extend(since.map(|start| (start, now)));
    spans
}
