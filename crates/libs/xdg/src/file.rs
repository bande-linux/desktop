use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

pub fn replace_file(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut staging = OsString::from(path.as_os_str());
    staging.push(".new");
    fs::write(&staging, contents)?;
    fs::rename(&staging, path)
}
