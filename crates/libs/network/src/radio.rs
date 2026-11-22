use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::PathBuf;

const RFKILL_DEVICE: &str = "/dev/rfkill";
const RFKILL_CLASS: &str = "/sys/class/rfkill";
const SAVED: &str = "bande/airplane-radios";
const EVENT_SIZE: usize = 8;
const TYPE_ALL: u8 = 0;
const OP_CHANGE_ALL: u8 = 3;
const TYPES: [(&str, u8); 8] = [
    ("wlan", 1),
    ("bluetooth", 2),
    ("uwb", 3),
    ("wimax", 4),
    ("wwan", 5),
    ("gps", 6),
    ("fm", 7),
    ("nfc", 8),
];

struct Radio {
    kind: String,
    blocked: bool,
}

fn radios() -> io::Result<Vec<Radio>> {
    let entries = match fs::read_dir(RFKILL_CLASS) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut radios = Vec::new();
    for entry in entries {
        let folder = entry?.path();
        let read =
            |name: &str| fs::read_to_string(folder.join(name)).map(|text| text.trim().to_owned());
        radios.push(Radio {
            kind: read("type")?,
            blocked: read("soft")? == "1" || read("hard")? == "1",
        });
    }
    Ok(radios)
}

fn saved_path() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join(SAVED))
}

fn change_all(kind: u8, blocked: bool) -> io::Result<()> {
    let event = [0, 0, 0, 0, kind, OP_CHANGE_ALL, u8::from(blocked), 0];
    OpenOptions::new()
        .write(true)
        .open(RFKILL_DEVICE)?
        .write_all(&event)
}

/// Whether every radio is off, or `None` when this computer has no radio to turn off.
pub fn airplane_mode() -> io::Result<Option<bool>> {
    let radios = radios()?;
    Ok((!radios.is_empty()).then(|| radios.iter().all(|radio| radio.blocked)))
}

/// Turns every radio off, remembering which were on, or turns back on the radios that were on before.
pub fn set_airplane_mode(on: bool) -> io::Result<()> {
    let saved = saved_path().ok_or_else(|| io::Error::other("no state folder"))?;
    if on {
        let mut kinds: Vec<String> = radios()?
            .into_iter()
            .filter(|radio| !radio.blocked)
            .map(|radio| radio.kind)
            .collect();
        kinds.sort();
        kinds.dedup();
        bande_xdg::replace_file(&saved, kinds.join("\n"))?;
        return change_all(TYPE_ALL, true);
    }
    let kinds = match fs::read_to_string(&saved) {
        Ok(text) => text.lines().map(str::to_owned).collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            radios()?.into_iter().map(|radio| radio.kind).collect()
        }
        Err(error) => return Err(error),
    };
    let kinds: Vec<String> = kinds;
    if kinds.is_empty() {
        change_all(TYPE_ALL, false)?;
    }
    for (name, kind) in TYPES {
        if kinds.iter().any(|saved| saved == name) {
            change_all(kind, false)?;
        }
    }
    match fs::remove_file(&saved) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Calls `changed` with [`airplane_mode`], then again each time it changes, until `changed` returns false.
pub fn follow_airplane_mode(mut changed: impl FnMut(Option<bool>) -> bool) -> io::Result<()> {
    let mut events = File::open(RFKILL_DEVICE)?;
    let mut now = airplane_mode()?;
    if !changed(now) {
        return Ok(());
    }
    let mut event = [0; EVENT_SIZE];
    loop {
        if events.read(&mut event)? == 0 {
            return Ok(());
        }
        let then = std::mem::replace(&mut now, airplane_mode()?);
        if now != then && !changed(now) {
            return Ok(());
        }
    }
}
