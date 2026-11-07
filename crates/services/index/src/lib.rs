mod background;
mod bus;
mod import;
mod sandbox;
mod service;
mod store;
mod tokens;

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

pub use import::import_stdin;

fn mount_of(path: &Path) -> Option<u64> {
    background::inspect(path).map(|inspected| inspected.mount)
}

fn index_folder(home: &Path) -> PathBuf {
    let own = home.join(".cache/bande/search");
    bande_xdg::cache_home()
        .map(|cache| cache.join("bande/search"))
        .filter(|chosen| {
            let parent = chosen.ancestors().find(|at| at.exists());
            parent.and_then(mount_of) == mount_of(home)
        })
        .unwrap_or(own)
}

pub fn serve() -> io::Result<()> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("HOME is not set"))?;
    let folder = index_folder(&home);
    if let Some(parent) = folder.parent() {
        fs::create_dir_all(parent)?;
    }
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&folder)?;
    service::run(home, folder)
}
