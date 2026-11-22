use std::fs;
use std::io::{self, Write};
use std::path::Path;

const RFKILL_CLASS: &str = "/sys/class/rfkill";
const RFKILL_DEVICE: &str = "/dev/rfkill";
const RFKILL_TYPE_ALL: u8 = 0;
const RFKILL_OP_CHANGE_ALL: u8 = 3;

fn soft_blocks(class: &Path) -> io::Result<Vec<bool>> {
    let mut blocks = Vec::new();
    for entry in fs::read_dir(class)? {
        let soft = fs::read_to_string(entry?.path().join("soft"))?;
        blocks.push(soft.trim() == "1");
    }
    Ok(blocks)
}

/// Whether every radio is switched off, or `None` when there are no radios.
pub fn airplane_mode() -> io::Result<Option<bool>> {
    let blocks = match soft_blocks(Path::new(RFKILL_CLASS)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        blocks => blocks?,
    };
    Ok((!blocks.is_empty()).then(|| blocks.iter().all(|blocked| *blocked)))
}

/// Switches every radio off, or back on, through `/dev/rfkill`.
pub fn set_airplane_mode(on: bool) -> io::Result<()> {
    let event = [
        0,
        0,
        0,
        0,
        RFKILL_TYPE_ALL,
        RFKILL_OP_CHANGE_ALL,
        u8::from(on),
        0,
    ];
    fs::OpenOptions::new()
        .write(true)
        .open(RFKILL_DEVICE)?
        .write_all(&event)
}
