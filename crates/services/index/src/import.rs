use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use bande_content::{Importer, confine, worker_program};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::sandbox::only_compute_from_now_on;

pub const MAX_TEXT: usize = 4 << 20;
const MAX_DOCUMENT: u64 = 256 << 20;
const SNIFFED: usize = 4096;
const IMPORT_TIME: Duration = Duration::from_secs(15);

pub fn import_stdin(importer: Importer) -> ExitCode {
    confine();
    let mut bytes = Vec::new();
    if io::stdin()
        .lock()
        .take(MAX_DOCUMENT)
        .read_to_end(&mut bytes)
        .is_err()
        || !only_compute_from_now_on()
    {
        return ExitCode::FAILURE;
    }
    let text = importer.import(bytes, MAX_TEXT);
    match io::stdout().lock().write_all(text.as_bytes()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

pub fn open_quietly(path: &Path) -> io::Result<File> {
    let flags = libc::O_NONBLOCK | libc::O_NOFOLLOW;
    OpenOptions::new()
        .read(true)
        .custom_flags(flags | libc::O_NOATIME)
        .open(path)
        .or_else(|_| OpenOptions::new().read(true).custom_flags(flags).open(path))
}

fn extension(path: &Path) -> Option<String> {
    Some(path.extension()?.to_str()?.to_ascii_lowercase())
}

fn plain_text(mut file: File) -> Option<String> {
    let mut bytes = Vec::new();
    (&mut file)
        .take(SNIFFED as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    let start = bytes.len();
    let two_bytes_wide = matches!(bytes.get(..2), Some([0xff, 0xfe] | [0xfe, 0xff]));
    if !two_bytes_wide && bytes.contains(&0) {
        return None;
    }
    file.take((MAX_TEXT - start) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if let Some(text) = utf16(&bytes) {
        return Some(text);
    }
    match String::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(error) => {
            let valid = error.utf8_error().valid_up_to();
            if valid < start.saturating_sub(4) {
                return None;
            }
            let mut bytes = error.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).ok()
        }
    }
}

fn utf16(bytes: &[u8]) -> Option<String> {
    let little = match bytes.get(..2)? {
        [0xff, 0xfe] => true,
        [0xfe, 0xff] => false,
        _ => return None,
    };
    let (pairs, _) = bytes.get(2..)?.as_chunks::<2>();
    let units = pairs.iter().map(|pair| match little {
        true => u16::from_le_bytes(*pair),
        false => u16::from_be_bytes(*pair),
    });
    Some(
        char::decode_utf16(units)
            .map(|unit| unit.unwrap_or(' '))
            .collect(),
    )
}

fn imported_by_child(importer: Importer, file: File) -> Option<String> {
    let mut child = Command::new(worker_program())
        .arg("--import")
        .arg(importer.keyword())
        .stdin(Stdio::from(file))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut output = child.stdout.take()?;
    let begun = Instant::now();
    let mut text = Vec::new();
    let mut buffer = [0u8; 65536];
    let finished = loop {
        let Some(left) = IMPORT_TIME.checked_sub(begun.elapsed()) else {
            break false;
        };
        let timeout = Timespec::try_from(left).ok()?;
        let mut fds = [PollFd::new(&output, PollFlags::IN)];
        if poll(&mut fds, Some(&timeout)).is_err() {
            break false;
        }
        match output.read(&mut buffer) {
            Ok(0) => break true,
            Ok(count) => {
                text.extend_from_slice(buffer.get(..count)?);
                if text.len() >= MAX_TEXT {
                    break true;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break false,
        }
    };
    let _ = child.kill();
    let exited = child.wait().ok()?;
    (finished && (exited.success() || text.len() >= MAX_TEXT))
        .then(|| String::from_utf8_lossy(&text).into_owned())
}

pub fn text_of(path: &Path, file: File) -> Option<String> {
    let importer = extension(path).and_then(|extension| Importer::for_extension(&extension));
    match importer {
        Some(importer) => imported_by_child(importer, file),
        None => plain_text(file),
    }
}
