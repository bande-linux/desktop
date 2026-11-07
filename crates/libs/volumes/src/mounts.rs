use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use rustix::event::{PollFd, PollFlags, poll};

const SYSTEM_MOUNTS: &str = "/run/bande/volumes";

pub fn user_mounts_root() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(Path::new(&runtime).join("bande/volumes"))
}

pub fn system_mounts_root(uid: u32) -> PathBuf {
    Path::new(SYSTEM_MOUNTS).join(uid.to_string())
}

pub fn connected_roots() -> Vec<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    user_mounts_root()
        .into_iter()
        .chain([system_mounts_root(uid)])
        .collect()
}

static CONNECTED_ROOTS: LazyLock<Vec<PathBuf>> = LazyLock::new(connected_roots);

pub fn on_connected_volume(path: &Path) -> bool {
    CONNECTED_ROOTS.iter().any(|root| path.starts_with(root))
}

pub fn quick_is_dir(path: &Path) -> bool {
    on_connected_volume(path) || path.is_dir()
}

pub fn connected_volume_of(path: &Path) -> Option<PathBuf> {
    CONNECTED_ROOTS.iter().find_map(|root| {
        let name = path.strip_prefix(root).ok()?.components().next()?;
        Some(root.join(name))
    })
}

pub fn is_connected(point: &Path) -> bool {
    connected_roots()
        .iter()
        .any(|root| point.parent() == Some(root.as_path()))
}

pub fn media_roots(user: &str) -> Vec<PathBuf> {
    let mut roots = vec![
        Path::new("/run/media").join(user),
        Path::new("/media").join(user),
    ];
    roots.extend(connected_roots());
    roots
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mounted {
    pub point: PathBuf,
    pub fs_type: String,
    pub source: String,
}

pub fn mounted(mountinfo: &str) -> impl Iterator<Item = Mounted> + '_ {
    mountinfo.lines().filter_map(|line| {
        let point = line.split(' ').nth(4)?;
        let (_, tail) = line.split_once(" - ")?;
        let mut tail = tail.split(' ');
        Some(Mounted {
            point: PathBuf::from(unescape(point)),
            fs_type: tail.next()?.to_owned(),
            source: unescape(tail.next()?),
        })
    })
}

pub fn volume_icon(mountinfo: &str, point: &Path) -> &'static str {
    if !is_connected(point) {
        return "drive-removable-media";
    }
    let fs_type = mounted(mountinfo)
        .find(|mount| mount.point == point)
        .map(|mount| mount.fs_type);
    match fs_type.as_deref() {
        Some("fuse.gphotofs") => "camera-photo",
        Some(kind) if kind.contains("mtpfs") => "phone",
        _ => "folder-remote",
    }
}

fn unescape(field: &str) -> String {
    let mut out = Vec::with_capacity(field.len());
    let bytes = field.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let octal = bytes
            .get(index + 1..index + 4)
            .filter(|_| bytes[index] == b'\\')
            .and_then(|digits| std::str::from_utf8(digits).ok())
            .and_then(|digits| u8::from_str_radix(digits, 8).ok());
        match octal {
            Some(byte) => {
                out.push(byte);
                index += 4;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn mounted_volumes(mountinfo: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut volumes: Vec<PathBuf> = mountinfo
        .lines()
        .filter_map(|line| line.split(' ').nth(4))
        .map(|field| PathBuf::from(unescape(field)))
        .filter(|point| {
            roots
                .iter()
                .any(|root| point.parent() == Some(root.as_path()))
        })
        .collect();
    volumes.sort();
    volumes.dedup();
    volumes
}

pub fn follow_mounts(changed: impl Fn() + Send + 'static) -> std::io::Result<()> {
    let table = std::fs::File::open("/proc/self/mountinfo")?;
    std::thread::Builder::new()
        .name("mount table".to_owned())
        .spawn(move || {
            let mut watched = [PollFd::new(&table, PollFlags::PRI)];
            while poll(&mut watched, None).is_ok() {
                changed();
            }
        })?;
    Ok(())
}

fn saved_file() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/state")))?;
    Some(state.join("bande/servers"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedServer {
    pub url: String,
    pub user: String,
    pub reconnect: bool,
}

pub fn load_saved_servers() -> Vec<SavedServer> {
    let text = saved_file()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .unwrap_or_default();
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let reconnect = fields.next()? == "1";
            let url = fields.next()?.to_owned();
            let user = fields.next().unwrap_or_default().to_owned();
            Some(SavedServer {
                url,
                user,
                reconnect,
            })
        })
        .collect()
}

pub fn store_saved_servers(saved: &[SavedServer]) {
    let Some(file) = saved_file() else {
        return;
    };
    let text: String = saved
        .iter()
        .map(|entry| {
            let flag = u8::from(entry.reconnect);
            format!("{flag}\t{}\t{}\n", entry.url, entry.user)
        })
        .collect();
    let written = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let scratch = file.with_extension("new");
            std::fs::write(&scratch, text)?;
            std::fs::rename(scratch, &file)
        });
    if let Err(error) = written {
        tracing::warn!("cannot save the server list: {error}");
    }
}
