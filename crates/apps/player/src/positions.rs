use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const KEPT: usize = 200;
const EDGE: Duration = Duration::from_secs(15);

pub struct Positions {
    file: Option<PathBuf>,
    entries: Vec<(PathBuf, u64)>,
    changed: bool,
}

impl Positions {
    pub fn load(file: Option<PathBuf>) -> Self {
        let file = file.filter(|file| remembered(file));
        let text = match file.as_deref().map(std::fs::read) {
            Some(Ok(text)) => text,
            Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => {
                tracing::warn!("cannot read where playback stopped: {error}");
                Vec::new()
            }
            _ => Vec::new(),
        };
        let entries = text
            .split(|byte| *byte == b'\n')
            .filter_map(|line| {
                let tab = line.iter().position(|byte| *byte == b'\t')?;
                let milliseconds = std::str::from_utf8(&line[..tab]).ok()?.parse().ok()?;
                let path = PathBuf::from(OsStr::from_bytes(&line[tab + 1..]));
                path.is_absolute().then_some((path, milliseconds))
            })
            .take(KEPT)
            .collect();
        Self {
            file,
            entries,
            changed: false,
        }
    }

    pub fn start(&self, path: &Path) -> u64 {
        self.entries
            .iter()
            .find(|(known, _)| known == path)
            .map_or(0, |(_, milliseconds)| *milliseconds)
    }

    pub fn remember(&mut self, path: &Path, position: Duration, length: Option<Duration>) {
        let before = self.entries.len();
        self.entries.retain(|(known, _)| known != path);
        let worth = length.is_some_and(|length| position >= EDGE && position + EDGE < length);
        let named = !path.as_os_str().as_bytes().contains(&b'\n');
        if worth && named {
            self.entries
                .insert(0, (path.to_path_buf(), position.as_millis() as u64));
            self.entries.truncate(KEPT);
        }
        self.changed |= worth || self.entries.len() != before;
    }

    pub fn save(&mut self) {
        if !std::mem::take(&mut self.changed) {
            return;
        }
        let Some(file) = self.file.as_deref().filter(|file| remembered(file)) else {
            return;
        };
        let mut text = Vec::new();
        for (path, milliseconds) in &self.entries {
            text.extend_from_slice(milliseconds.to_string().as_bytes());
            text.push(b'\t');
            text.extend_from_slice(path.as_os_str().as_bytes());
            text.push(b'\n');
        }
        if let Err(error) = bande_xdg::replace_private_file(file, text) {
            tracing::warn!("cannot keep where playback stopped: {error}");
        }
    }
}

fn remembered(file: &Path) -> bool {
    if bande_search::remembers_recent_files() {
        return true;
    }
    match std::fs::remove_file(file) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            tracing::warn!("cannot forget where playback stopped: {error}");
        }
        _ => {}
    }
    false
}
