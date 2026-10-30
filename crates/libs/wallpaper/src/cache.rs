use std::fs::{self, File};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::choice::Fit;
use crate::picture::{Picture, RenderError, render};

const KEPT: usize = 4;
const EXTENSION: &str = "rgba";

fn entry(cache: &Path, path: &Path, fit: Fit, size: (u32, u32)) -> Option<PathBuf> {
    let metadata = fs::metadata(path).ok()?;
    let mut hasher = DefaultHasher::new();
    (path, metadata.len(), metadata.modified().ok()?, fit, size).hash(&mut hasher);
    Some(cache.join(format!("{:016x}.{EXTENSION}", hasher.finish())))
}

fn load(entry: &Path) -> io::Result<Picture> {
    let mut file = File::open(entry)?;
    let mut header = [0; 8];
    file.read_exact(&mut header)?;
    let [width, height] = [&header[..4], &header[4..]]
        .map(|half| u32::from_le_bytes([half[0], half[1], half[2], half[3]]));
    let length = width as usize * height as usize * 4;
    if file.metadata()?.len() != (header.len() + length) as u64 {
        return Err(io::Error::other(
            "the cached wallpaper has the wrong length",
        ));
    }
    let mut rgba = vec![0; length];
    file.read_exact(&mut rgba)?;
    let _ = file.set_modified(SystemTime::now());
    Ok(Picture {
        width,
        height,
        rgba,
    })
}

fn store(entry: &Path, picture: &Picture) -> io::Result<()> {
    let folder = entry
        .parent()
        .ok_or_else(|| io::Error::other("the cache has no folder"))?;
    fs::create_dir_all(folder)?;
    let staging = entry.with_extension(format!("{}.partial", std::process::id()));
    let written = File::create(&staging).and_then(|mut file| {
        file.write_all(&picture.width.to_le_bytes())?;
        file.write_all(&picture.height.to_le_bytes())?;
        file.write_all(&picture.rgba)
    });
    match written.and_then(|()| fs::rename(&staging, entry)) {
        Ok(()) => prune(folder),
        Err(error) => {
            let _ = fs::remove_file(&staging);
            Err(error)
        }
    }
}

fn prune(folder: &Path) -> io::Result<()> {
    let mut entries: Vec<(SystemTime, PathBuf)> = fs::read_dir(folder)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == EXTENSION)
        })
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    entries.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, stale) in entries.into_iter().skip(KEPT) {
        fs::remove_file(stale)?;
    }
    Ok(())
}

pub fn render_cached(
    cache: &Path,
    path: &Path,
    fit: Fit,
    width: u32,
    height: u32,
) -> Result<Picture, RenderError> {
    let entry = entry(cache, path, fit, (width, height));
    if let Some(picture) = entry.as_deref().and_then(|entry| load(entry).ok()) {
        return Ok(picture);
    }
    let picture = render(path, fit, width, height)?;
    if let Some(entry) = entry
        && let Err(error) = store(&entry, &picture)
    {
        tracing::info!("cannot keep the wallpaper for the next sign-in: {error}");
    }
    Ok(picture)
}
