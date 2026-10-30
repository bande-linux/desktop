use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::time::SystemTime;

use unbound_fs::Entry;
use unbound_thumbnails::{Request, ThumbnailSize, Thumbnails};
use unbound_toolkit::Image;

const KEPT: usize = 1500;

type Key = (PathBuf, ThumbnailSize);

#[derive(Default)]
pub(crate) struct Thumbs {
    service: Option<Thumbnails>,
    pictures: HashMap<Key, (Option<SystemTime>, Option<Image>)>,
    asked: HashMap<Key, Option<SystemTime>>,
    wanted: Vec<Key>,
}

impl Thumbs {
    pub(crate) fn set_service(&mut self, service: Thumbnails) {
        self.service = Some(service);
        self.pictures.clear();
        self.asked.clear();
        self.wanted.clear();
    }

    pub(crate) fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.service.as_ref().map(Thumbnails::fd)
    }

    fn fresh(&self, entry: &Entry, size: ThumbnailSize) -> Option<&Option<Image>> {
        self.pictures
            .get(&(entry.path.clone(), size))
            .filter(|(modified, _)| *modified == entry.modified)
            .map(|(_, image)| image)
    }

    pub(crate) fn get(&self, entry: &Entry, size: ThumbnailSize) -> Option<Image> {
        self.fresh(entry, size).cloned().flatten()
    }

    pub(crate) fn want<'a>(
        &mut self,
        entries: impl IntoIterator<Item = (&'a Entry, ThumbnailSize)>,
        mime: impl Fn(&Entry) -> String,
    ) {
        let Some(service) = &self.service else {
            return;
        };
        let missing = entries
            .into_iter()
            .filter(|(entry, size)| !entry.is_folder() && self.fresh(entry, *size).is_none())
            .collect::<Vec<_>>();
        let keys = missing
            .iter()
            .map(|(entry, size)| (entry.path.clone(), *size))
            .collect::<Vec<_>>();
        if keys == self.wanted {
            return;
        }
        let requests = missing
            .iter()
            .map(|(entry, size)| Request {
                path: entry.path.clone(),
                mime: mime(entry),
                size: *size,
            })
            .collect();
        for (entry, size) in &missing {
            self.asked
                .insert((entry.path.clone(), *size), entry.modified);
        }
        service.want(requests);
        self.wanted = keys;
    }

    pub(crate) fn take(&mut self) -> bool {
        let Some(service) = &self.service else {
            return false;
        };
        let done = service.take();
        if done.is_empty() {
            return false;
        }
        if self.pictures.len() + done.len() > KEPT {
            let wanted = &self.wanted;
            self.pictures.retain(|key, _| wanted.contains(key));
        }
        for thumbnailed in done {
            let key = (thumbnailed.path, thumbnailed.size);
            let modified = self.asked.remove(&key).flatten();
            let image = thumbnailed
                .picture
                .and_then(|picture| Image::from_rgba(picture.width, picture.height, picture.rgba));
            self.wanted.retain(|wanted| *wanted != key);
            self.pictures.insert(key, (modified, image));
        }
        true
    }
}
