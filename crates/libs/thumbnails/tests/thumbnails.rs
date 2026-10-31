#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use bande_thumbnails::{
    Cache, Request, ThumbnailSize, Thumbnailed, Thumbnailers, Thumbnails, modified, thumbnail, uri,
};
use tempfile::TempDir;

struct Place {
    root: TempDir,
}

impl Place {
    fn new() -> Self {
        Self {
            root: TempDir::new().expect("temporary folder"),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn cache(&self) -> Cache {
        Cache::new(self.path("cache/thumbnails"))
    }

    fn image(&self, name: &str, width: u32, height: u32) -> PathBuf {
        let path = self.path(name);
        image::RgbaImage::from_pixel(width, height, image::Rgba([200, 40, 40, 255]))
            .save(&path)
            .expect("image");
        path
    }

    fn thumbnailer(&self, mime: &str, delay: &str) -> Thumbnailers {
        let sample = self.image("sample.png", 600, 300);
        let script = self.path("fake-thumbnailer");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nsleep {delay}\necho \"$1 $2 $3 $4\" >> {log}\ncp {sample} \"$4\"\n",
                log = self.path("calls").display(),
                sample = sample.display()
            ),
        )
        .expect("script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("executable");
        let data = self.path("data");
        fs::create_dir_all(data.join("thumbnailers")).expect("folder");
        fs::write(
            data.join("thumbnailers/fake.thumbnailer"),
            format!(
                "[Thumbnailer Entry]\nTryExec={script}\nExec={script} -s %s %u %o\nMimeType={mime};\n",
                script = script.display()
            ),
        )
        .expect("thumbnailer");
        fs::write(
            data.join("thumbnailers/missing.thumbnailer"),
            "[Thumbnailer Entry]\nTryExec=/nonexistent/program\nExec=/nonexistent/program %i %o\nMimeType=video/mp4;\n",
        )
        .expect("thumbnailer");
        Thumbnailers::load(&[data])
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.path("calls"))
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn touch(path: &Path) {
    let later = SystemTime::now() + Duration::from_secs(3600);
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open")
        .set_modified(later)
        .expect("touch");
}

#[test]
fn an_image_is_scaled_into_the_shared_cache_and_reused_until_it_changes() {
    let place = Place::new();
    let cache = place.cache();
    let photo = place.image("photo.png", 1000, 500);
    let made = thumbnail(
        &cache,
        &Thumbnailers::default(),
        &photo,
        "image/png",
        ThumbnailSize::Normal,
    )
    .expect("thumbnail");
    assert_eq!((made.width, made.height), (128, 64));
    let stored = cache.path(&uri(&photo), ThumbnailSize::Normal);
    assert!(stored.starts_with(place.path("cache/thumbnails/normal")));
    let mode = fs::metadata(&stored).expect("stored").permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let mtime = modified(&photo).expect("mtime");
    assert_eq!(
        cache.load(&uri(&photo), mtime, ThumbnailSize::Normal),
        Some(made)
    );
    touch(&photo);
    assert_eq!(
        cache.load(
            &uri(&photo),
            modified(&photo).expect("mtime"),
            ThumbnailSize::Normal
        ),
        None
    );
    let large = thumbnail(
        &cache,
        &Thumbnailers::default(),
        &photo,
        "image/png",
        ThumbnailSize::Large,
    )
    .expect("large thumbnail");
    assert_eq!((large.width, large.height), (256, 128));
}

#[test]
fn a_broken_image_is_remembered_as_failed_until_it_changes() {
    let place = Place::new();
    let cache = place.cache();
    let broken = place.path("broken.png");
    fs::write(&broken, b"not a picture").expect("file");
    let none = Thumbnailers::default();
    assert_eq!(
        thumbnail(&cache, &none, &broken, "image/png", ThumbnailSize::Normal),
        None
    );
    assert!(cache.has_failed(&uri(&broken), modified(&broken).expect("mtime")));
    place.image("broken.png", 10, 10);
    touch(&broken);
    assert!(!cache.has_failed(&uri(&broken), modified(&broken).expect("mtime")));
    assert!(thumbnail(&cache, &none, &broken, "image/png", ThumbnailSize::Normal).is_some());
}

#[test]
fn files_nothing_can_draw_are_neither_drawn_nor_marked_failed() {
    let place = Place::new();
    let cache = place.cache();
    let notes = place.path("notes.txt");
    fs::write(&notes, "hello").expect("file");
    let thumbnailers = place.thumbnailer("application/pdf", "0");
    assert_eq!(
        thumbnail(
            &cache,
            &thumbnailers,
            &notes,
            "text/plain",
            ThumbnailSize::Normal
        ),
        None
    );
    assert!(!cache.has_failed(&uri(&notes), modified(&notes).expect("mtime")));
    assert!(place.calls().is_empty());
}

#[test]
fn installed_thumbnailers_draw_other_kinds_with_the_spec_arguments() {
    let place = Place::new();
    let cache = place.cache();
    let paper = place.path("paper one.pdf");
    fs::write(&paper, "%PDF-1.7").expect("file");
    let thumbnailers = place.thumbnailer("application/pdf", "0");
    let made = thumbnail(
        &cache,
        &thumbnailers,
        &paper,
        "application/pdf",
        ThumbnailSize::Large,
    )
    .expect("thumbnail");
    assert_eq!((made.width, made.height), (256, 128));
    let calls = place.calls();
    assert_eq!(calls.len(), 1);
    let arguments = calls[0].split(' ').collect::<Vec<_>>();
    assert_eq!(arguments[0], "-s");
    assert_eq!(arguments[1], "256");
    assert_eq!(arguments[2], uri(&paper));
    assert!(Path::new(arguments[3]).starts_with(cache.root()));
    assert!(!Path::new(arguments[3]).exists());
    thumbnail(
        &cache,
        &thumbnailers,
        &paper,
        "application/pdf",
        ThumbnailSize::Large,
    )
    .expect("cached thumbnail");
    assert_eq!(place.calls().len(), 1);
    let video = place.path("clip.mp4");
    fs::write(&video, "video").expect("file");
    assert_eq!(
        thumbnail(
            &cache,
            &thumbnailers,
            &video,
            "video/mp4",
            ThumbnailSize::Normal
        ),
        None
    );
}

fn collect(thumbnails: &mut Thumbnails, wanted: usize, within: Duration) -> Vec<Thumbnailed> {
    let deadline = Instant::now() + within;
    let mut done = Vec::new();
    while Instant::now() < deadline && done.len() < wanted {
        done.extend(thumbnails.take());
        std::thread::sleep(Duration::from_millis(10));
    }
    done
}

fn request(path: &Path, mime: &str) -> Request {
    Request {
        path: path.to_path_buf(),
        mime: mime.to_owned(),
        size: ThumbnailSize::Normal,
    }
}

#[test]
fn the_worker_makes_wanted_thumbnails_and_drops_those_no_longer_wanted() {
    let place = Place::new();
    let thumbnailers = place.thumbnailer("application/pdf", "0.4");
    let papers = (0..6)
        .map(|index| {
            let path = place.path(&format!("paper{index}.pdf"));
            fs::write(&path, format!("%PDF {index}")).expect("file");
            path
        })
        .collect::<Vec<_>>();
    let mut thumbnails = Thumbnails::spawn(place.cache(), thumbnailers).expect("worker");
    thumbnails.want(
        papers
            .iter()
            .map(|path| request(path, "application/pdf"))
            .collect(),
    );
    std::thread::sleep(Duration::from_millis(150));
    let photo = place.image("photo.png", 300, 300);
    thumbnails.want(vec![request(&photo, "image/png")]);
    let done = collect(&mut thumbnails, 3, Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(600));
    let mut done = done
        .into_iter()
        .chain(thumbnails.take())
        .collect::<Vec<_>>();
    done.sort_by(|a, b| a.path.cmp(&b.path));
    let paths = done
        .iter()
        .map(|done| done.path.clone())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec![papers[0].clone(), papers[1].clone(), photo]);
    assert!(done.iter().all(|done| done.picture.is_some()));
}
