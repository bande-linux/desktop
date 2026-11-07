use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const SNAPSHOT_NAME: &str = ".bande-backup-snapshot";
const SNAPSHOTS: &str = "/.snapshots";
const VIEW: &str = "/run/bande-backup/view";
const BACKED_UP_FILESYSTEMS: [&str; 5] = ["ext4", "ext3", "btrfs", "xfs", "f2fs"];
const NEVER_UNDER: [&str; 6] = ["/run", "/media", "/mnt", "/tmp", "/proc", "/sys"];
pub const EXCLUDED: [&str; 20] = [
    SNAPSHOT_NAME,
    "/dev",
    "/proc",
    "/sys",
    "/run",
    "/tmp",
    "/mnt",
    "/media",
    "/lost+found",
    "/var/tmp",
    "/var/cache",
    "/var/lib/systemd/coredump",
    "/var/log/journal",
    "/swapfile",
    "/swap.img",
    "/swap/*",
    "/home/*/.cache",
    "/home/*/.local/share/Trash",
    "/root/.cache",
    SNAPSHOTS,
];

fn unescaped(field: &str) -> String {
    field
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\134", "\\")
}

pub fn sources_in(mounts: &str) -> Vec<PathBuf> {
    let mut sources = vec![PathBuf::from("/")];
    for line in mounts.lines() {
        let mut fields = line.split(' ');
        let (Some(device), Some(point), Some(kind)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let point = PathBuf::from(unescaped(point));
        let eligible = device.starts_with('/')
            && BACKED_UP_FILESYSTEMS.contains(&kind)
            && !NEVER_UNDER.iter().any(|root| point.starts_with(root))
            && !EXCLUDED.iter().any(|excluded| point == Path::new(excluded))
            && !sources.contains(&point);
        if eligible {
            sources.push(point);
        }
    }
    sources.sort();
    sources
}

pub fn sources() -> Vec<PathBuf> {
    match fs::read_to_string("/proc/self/mounts") {
        Ok(mounts) => sources_in(&mounts),
        Err(error) => {
            tracing::warn!("cannot read the mount table: {error}");
            vec![PathBuf::from("/")]
        }
    }
}

pub fn backup_args(host: &str, sources: &[PathBuf]) -> Vec<String> {
    let mut args = vec![
        "backup".to_owned(),
        "--json".to_owned(),
        "--one-file-system".to_owned(),
        "--exclude-caches".to_owned(),
        "--host".to_owned(),
        host.to_owned(),
    ];
    for path in EXCLUDED {
        args.push("--exclude".to_owned());
        args.push(path.to_owned());
    }
    args.extend(
        sources
            .iter()
            .map(|path| path.to_string_lossy().into_owned()),
    );
    args
}

fn is_btrfs(point: &Path, mounts: &str) -> bool {
    mounts.lines().rev().any(|line| {
        let mut fields = line.split(' ').skip(1);
        fields.next().map(unescaped).as_deref().map(Path::new) == Some(point)
            && fields.next() == Some("btrfs")
    })
}

fn snapshot_of(source: &Path) -> PathBuf {
    let snapshots = Path::new(SNAPSHOTS);
    let shared =
        fs::read_to_string("/proc/self/mounts").is_ok_and(|mounts| is_btrfs(snapshots, &mounts));
    if shared {
        let name = source.to_string_lossy().replace('/', "-");
        snapshots.join("home").join(format!("backup{name}"))
    } else {
        source.join(SNAPSHOT_NAME)
    }
}

fn delete_snapshot(path: &Path) {
    if !path.exists() {
        return;
    }
    let deleted = Command::new("btrfs")
        .args(["subvolume", "delete"])
        .arg(path)
        .stdout(Stdio::null())
        .status();
    if !deleted.is_ok_and(|status| status.success()) {
        tracing::warn!("cannot delete the snapshot {}", path.display());
    }
}

fn take_snapshot(source: &Path) -> bool {
    let path = snapshot_of(source);
    delete_snapshot(&path);
    if let Some(parent) = path.parent()
        && let Err(error) = fs::create_dir_all(parent)
    {
        tracing::warn!("cannot make {}: {error}", parent.display());
        return false;
    }
    Command::new("btrfs")
        .args(["subvolume", "snapshot", "-r"])
        .arg(source)
        .arg(&path)
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn view_script(sources: &[PathBuf], snapshots: &[PathBuf], disk: &Path) -> String {
    let view = Path::new(VIEW);
    let shown = |source: &Path| -> PathBuf {
        if snapshots.iter().any(|known| known == source) {
            snapshot_of(source)
        } else {
            source.to_path_buf()
        }
    };
    let inside = |source: &Path| view.join(source.strip_prefix("/").unwrap_or(source));
    let mut script = format!("set -eu\nmkdir -p {VIEW}\n");
    for source in sources {
        script.push_str(&format!(
            "mount --bind {} {}\n",
            quoted(&shown(source)),
            quoted(&inside(source))
        ));
    }
    for pseudo in ["/dev", "/proc", "/sys", "/var/cache"] {
        script.push_str(&format!(
            "mount --rbind {pseudo} {}\n",
            quoted(&inside(Path::new(pseudo)))
        ));
    }
    let inner_disk = inside(disk);
    script.push_str(&format!(
        "mount -t tmpfs -o mode=0700 none {run}\nmkdir -p {disk}\nmount --bind {outer} {disk}\nexec chroot {VIEW} \"$@\"\n",
        run = quoted(&inside(Path::new("/run"))),
        disk = quoted(&inner_disk),
        outer = quoted(disk),
    ));
    script
}

pub struct SnapshotView {
    snapshots: Vec<PathBuf>,
    script: String,
}

impl SnapshotView {
    pub fn take(sources: &[PathBuf], disk: &Path) -> Option<Self> {
        let mounts = fs::read_to_string("/proc/self/mounts").ok()?;
        let on_btrfs: Vec<PathBuf> = sources
            .iter()
            .filter(|source| is_btrfs(source, &mounts))
            .cloned()
            .collect();
        if on_btrfs.is_empty() {
            return None;
        }
        let mut taken = Vec::new();
        for source in &on_btrfs {
            if !take_snapshot(source) {
                tracing::warn!("cannot snapshot {}; reading it live", source.display());
                continue;
            }
            taken.push(source.clone());
        }
        if taken.is_empty() {
            return None;
        }
        let script = view_script(sources, &taken, disk);
        Some(Self {
            snapshots: taken,
            script,
        })
    }

    pub fn wrapper(&self) -> Vec<String> {
        [
            "unshare",
            "--mount",
            "--propagation",
            "private",
            "sh",
            "-c",
            self.script.as_str(),
            "sh",
        ]
        .map(str::to_owned)
        .to_vec()
    }
}

impl Drop for SnapshotView {
    fn drop(&mut self) {
        for source in &self.snapshots {
            delete_snapshot(&snapshot_of(source));
        }
    }
}
