use std::io;
use std::path::{Path, PathBuf};

pub fn autostart_dir(config_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    config_home
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|home| home.join(".config")))
        .map(|config| config.join("autostart"))
}

pub fn opens_at_login(dir: &Path, entry_id: &str) -> bool {
    std::fs::read_to_string(dir.join(entry_id)).is_ok_and(|source| {
        !source
            .lines()
            .any(|line| line.replace(' ', "").eq_ignore_ascii_case("Hidden=true"))
    })
}

pub fn set_open_at_login(
    dir: &Path,
    entry_id: &str,
    desktop_file: &Path,
    open: bool,
) -> io::Result<()> {
    let target = dir.join(entry_id);
    if !open {
        return match std::fs::remove_file(&target) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir)?;
    std::fs::copy(desktop_file, target).map(drop)
}
