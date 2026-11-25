use std::io;
use std::path::Path;

use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

const USUAL_LIMIT: u64 = 8 << 20;
const MAPPINGS: &str = "/proc/self/maps";

struct Mapping {
    start: usize,
    length: usize,
    own_program: bool,
}

fn mapping(line: &str, program: Option<&Path>) -> Option<Mapping> {
    let mut fields = line.split_ascii_whitespace();
    let (start, end) = fields.next()?.split_once('-')?;
    let access = fields.next()?;
    let path = fields.nth(3).filter(|path| path.starts_with('/'))?;
    if access.contains('w') || !access.starts_with('r') || !access.ends_with('p') {
        return None;
    }
    let start = usize::from_str_radix(start, 16).ok()?;
    let end = usize::from_str_radix(end, 16).ok()?;
    Some(Mapping {
        start,
        length: end.checked_sub(start)?,
        own_program: program == Some(Path::new(path)),
    })
}

fn lock(mapping: &Mapping) -> io::Result<()> {
    // SAFETY: mlock2 reads and writes no memory of this process; a range that is no longer
    // mapped makes it fail.
    match unsafe {
        libc::mlock2(
            mapping.start as *const libc::c_void,
            mapping.length,
            libc::MLOCK_ONFAULT,
        )
    } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

fn lock_within(limit: Option<u64>) {
    let maps = match std::fs::read_to_string(MAPPINGS) {
        Ok(maps) => maps,
        Err(error) => return tracing::warn!("the program's code may be reread: {error}"),
    };
    let program = std::env::current_exe().ok();
    let mut mappings: Vec<Mapping> = maps
        .lines()
        .filter_map(|line| mapping(line, program.as_deref()))
        .collect();
    mappings.sort_by_key(|mapping| (!mapping.own_program, mapping.length));
    let mut left = limit.map_or(usize::MAX, |limit| limit as usize);
    for mapping in &mappings {
        if mapping.length > left {
            continue;
        }
        match lock(mapping) {
            Ok(()) => left -= mapping.length,
            Err(error) => {
                return tracing::debug!("not all of the program's code stays in memory: {error}");
            }
        }
    }
}

/// Keeps the pages of the program and its libraries that have been or will be used in memory, as
/// far as the process may lock memory, and then gives up the right to lock more than usual, which
/// the programs it starts would inherit. Their files are read again from the disk otherwise: the
/// pages belong to whichever program read them first, such as the login window's compositor, so
/// the memory kept for the session does not cover them.
pub fn keep_code_in_memory() {
    let limit = getrlimit(Resource::Memlock);
    lock_within(limit.current);
    if limit.maximum.is_none_or(|most| most > USUAL_LIMIT) {
        let usual = Rlimit {
            current: limit
                .current
                .map(|now| now.min(USUAL_LIMIT))
                .or(Some(USUAL_LIMIT)),
            maximum: Some(USUAL_LIMIT),
        };
        if let Err(error) = setrlimit(Resource::Memlock, usual) {
            tracing::warn!("apps may lock more memory than usual: {error}");
        }
    }
}
