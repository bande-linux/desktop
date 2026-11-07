use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use rustix::ioctl;

pub const SYSFS_DRM: &str = "/sys/class/drm";

const DDC_ADDRESS: usize = 0x37;
const I2C_SLAVE: ioctl::Opcode = 0x0703;
const DISPLAY_ADDRESS: u8 = 0x6E;
const HOST_ADDRESS: u8 = 0x51;
const REPLY_SEED: u8 = 0x50;
const LENGTH_FLAG: u8 = 0x80;
const GET_VCP: u8 = 0x01;
const GET_VCP_REPLY: u8 = 0x02;
const SET_VCP: u8 = 0x03;
const BRIGHTNESS: u8 = 0x10;
const REPLY_SIZE: usize = 11;
const REPLY_WAIT: Duration = Duration::from_millis(40);
const RETRY_WAIT: Duration = Duration::from_millis(100);
const WRITE_WAIT: Duration = Duration::from_millis(50);
const ATTEMPTS: usize = 3;

pub trait Wire: Send {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn receive(&mut self, bytes: &mut [u8]) -> io::Result<()>;
}

pub struct I2cWire {
    device: File,
}

impl I2cWire {
    pub fn open(path: &Path) -> io::Result<Self> {
        let device = OpenOptions::new().read(true).write(true).open(path)?;
        let target = unsafe { ioctl::IntegerSetter::<I2C_SLAVE>::new_usize(DDC_ADDRESS) };
        unsafe { ioctl::ioctl(device.as_fd(), target) }?;
        Ok(Self { device })
    }
}

impl Wire for I2cWire {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.device.write_all(bytes)
    }

    fn receive(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        self.device.read_exact(bytes)
    }
}

pub fn connector_bus(drm: &Path, devices: &Path, connector: &str) -> Option<PathBuf> {
    let suffix = format!("-{connector}");
    let directory = fs::read_dir(drm)
        .ok()?
        .filter_map(Result::ok)
        .find(|entry| entry.file_name().to_string_lossy().ends_with(&suffix))?
        .path();
    let adapter = fs::canonicalize(directory.join("ddc"))
        .ok()
        .and_then(|link| link.file_name().map(ToOwned::to_owned))
        .or_else(|| {
            fs::read_dir(&directory)
                .ok()?
                .filter_map(Result::ok)
                .map(|entry| entry.file_name())
                .find(|name| name.to_string_lossy().starts_with("i2c-"))
        })?;
    Some(devices.join(adapter))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub value: u16,
    pub maximum: u16,
}

fn checksum(seed: u8, bytes: &[u8]) -> u8 {
    bytes.iter().fold(seed, |sum, byte| sum ^ byte)
}

fn message(payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![HOST_ADDRESS, LENGTH_FLAG | payload.len() as u8];
    bytes.extend_from_slice(payload);
    bytes.push(checksum(DISPLAY_ADDRESS, &bytes));
    bytes
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

fn brightness_reply(reply: &[u8; REPLY_SIZE]) -> io::Result<Reading> {
    let [
        source,
        length,
        opcode,
        result,
        code,
        _,
        max_high,
        max_low,
        high,
        low,
        sum,
    ] = *reply;
    if source != DISPLAY_ADDRESS || checksum(REPLY_SEED, &reply[..REPLY_SIZE - 1]) != sum {
        return Err(invalid("the display's reply is garbled"));
    }
    if length != LENGTH_FLAG | 8 || opcode != GET_VCP_REPLY || code != BRIGHTNESS {
        return Err(invalid("the display is not ready to reply"));
    }
    if result != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the display has no brightness control",
        ));
    }
    let maximum = u16::from_be_bytes([max_high, max_low]);
    if maximum == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the display reports no brightness range",
        ));
    }
    Ok(Reading {
        value: u16::from_be_bytes([high, low]).min(maximum),
        maximum,
    })
}

pub struct Monitor {
    wire: Box<dyn Wire>,
}

impl Monitor {
    pub fn new(wire: Box<dyn Wire>) -> Self {
        Self { wire }
    }

    fn ask_brightness(&mut self) -> io::Result<Reading> {
        self.wire.send(&message(&[GET_VCP, BRIGHTNESS]))?;
        thread::sleep(REPLY_WAIT);
        let mut reply = [0; REPLY_SIZE];
        self.wire.receive(&mut reply)?;
        brightness_reply(&reply)
    }

    fn with_retries<T>(mut attempt: impl FnMut() -> io::Result<T>) -> io::Result<T> {
        let mut outcome = attempt();
        for _ in 1..ATTEMPTS {
            match &outcome {
                Err(error) if error.kind() != io::ErrorKind::Unsupported => {
                    thread::sleep(RETRY_WAIT);
                    outcome = attempt();
                }
                _ => break,
            }
        }
        outcome
    }

    pub fn brightness(&mut self) -> io::Result<Reading> {
        Self::with_retries(|| self.ask_brightness())
    }

    pub fn set_brightness(&mut self, value: u16) -> io::Result<()> {
        let [high, low] = value.to_be_bytes();
        let bytes = message(&[SET_VCP, BRIGHTNESS, high, low]);
        Self::with_retries(|| self.wire.send(&bytes))?;
        thread::sleep(WRITE_WAIT);
        Ok(())
    }
}
