use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use rustix::fs::XattrFlags;
use rustix::io::Errno;

use crate::naming::split_extension;

const ATTRIBUTE: &str = "user.bande.hide-extension";
const FALLBACK_FILE: &str = "bande/hidden-extensions";

type Remembered = (Option<SystemTime>, HashMap<PathBuf, bool>);

static FALLBACK: Mutex<Option<Remembered>> = Mutex::new(None);

fn fallback_file() -> Option<PathBuf> {
    bande_xdg::state_home().map(|state| state.join(FALLBACK_FILE))
}

fn parse(text: &str) -> HashMap<PathBuf, bool> {
    text.lines()
        .filter_map(|line| {
            let (hidden, path) = line.split_once('\t')?;
            Some((PathBuf::from(path), hidden == "1"))
        })
        .collect()
}

fn with_fallback<T>(use_it: impl FnOnce(&Path, &mut HashMap<PathBuf, bool>) -> T) -> Option<T> {
    let file = fallback_file()?;
    let modified = fs::metadata(&file)
        .and_then(|metadata| metadata.modified())
        .ok();
    let mut remembered = FALLBACK.lock().ok()?;
    let fresh = remembered
        .as_ref()
        .is_some_and(|(known, _)| *known == modified);
    if !fresh {
        let text = fs::read_to_string(&file).unwrap_or_default();
        *remembered = Some((modified, parse(&text)));
    }
    let (_, map) = remembered.as_mut()?;
    Some(use_it(&file, map))
}

pub fn extension_hidden(path: &Path) -> Option<bool> {
    let mut value = [0u8; 1];
    match rustix::fs::getxattr(path, ATTRIBUTE, &mut value) {
        Ok(1) => Some(value[0] == b'1'),
        Err(Errno::OPNOTSUPP) => with_fallback(|_, map| map.get(path).copied()).flatten(),
        _ => None,
    }
}

pub fn set_extension_hidden(path: &Path, hidden: bool) -> io::Result<()> {
    let value = if hidden { b"1" } else { b"0" };
    match rustix::fs::setxattr(path, ATTRIBUTE, value, XattrFlags::empty()) {
        Err(Errno::OPNOTSUPP) => with_fallback(|file, map| {
            map.insert(path.to_path_buf(), hidden);
            let text = map
                .iter()
                .filter(|(path, _)| path.exists())
                .map(|(path, hidden)| format!("{}\t{}\n", u8::from(*hidden), path.display()))
                .collect::<String>();
            bande_xdg::replace_file(file, text)
        })
        .unwrap_or_else(|| Err(io::Error::from(io::ErrorKind::Unsupported))),
        result => result.map_err(io::Error::from),
    }
}

pub fn hidden_extension<'a>(
    name: &'a str,
    known: Option<&'a str>,
    hidden: Option<bool>,
) -> Option<&'a str> {
    match hidden {
        Some(false) => None,
        Some(true) => {
            known.or_else(|| Some(split_extension(name, false).1).filter(|e| !e.is_empty()))
        }
        None => known,
    }
}
