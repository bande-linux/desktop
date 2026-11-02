use bande_fs::Entry;
use bande_quick_look::{QuickLookLink, Subject};
use bande_thumbnails::ThumbnailSize;
use bande_toolkit::Rect;
use bande_toolkit::ui::{Key, Modifiers};

use crate::L10N;
use crate::view::{FilesView, Hit, Request};

const ICON_PIXELS: u32 = 256;
const FOLDER_MIME: &str = "inode/directory";

impl FilesView {
    pub fn set_quick_look(&mut self, link: QuickLookLink) {
        self.quick_look = Some(link);
    }

    pub(crate) fn quick_look_shown(&self) -> bool {
        self.quick_look
            .as_ref()
            .is_some_and(|link| link.borrow().is_shown())
    }

    fn origin_of(&self, entry: &Entry) -> Option<Rect> {
        let active = self.browser.active();
        let row = self
            .browser
            .active_column()
            .entries
            .iter()
            .position(|shown| shown.path == entry.path)?;
        self.hits.iter().find_map(|(rect, hit)| match hit {
            Hit::Row { column, row: at } if *column == active && *at == row => Some(*rect),
            _ => None,
        })
    }

    fn subject(&mut self, entry: Entry) -> Subject {
        let (mime, kind) = match entry.is_folder() {
            true => (FOLDER_MIME.to_owned(), L10N.tr("folder")),
            false => {
                let mime = self.apps.mime_for(&entry.path);
                let kind = self.apps.kind(&mime);
                (mime, kind)
            }
        };
        let app = match entry.is_folder() || entry.trashed.is_some() {
            true => None,
            false => self.apps.default_for_path(&entry.path).cloned(),
        };
        let origin = self.origin_of(&entry);
        let icon = self
            .thumbs
            .get(&entry, ThumbnailSize::XLarge)
            .or_else(|| self.entry_icon(&entry, ICON_PIXELS));
        Subject {
            entry,
            mime,
            kind,
            app,
            icon,
            origin,
        }
    }

    pub(crate) fn close_quick_look(&mut self) {
        if let Some(link) = self.quick_look.as_ref().filter(|_| self.quick_look_shown()) {
            link.borrow_mut().close();
        }
    }

    pub(crate) fn toggle_quick_look(&mut self) {
        match self.quick_look_shown() {
            true => self.close_quick_look(),
            false => self.open_quick_look(false),
        }
    }

    pub(crate) fn open_quick_look(&mut self, full_screen: bool) {
        let Some(link) = self.quick_look.clone() else {
            return;
        };
        let selection = self
            .browser
            .selection()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let Some(entry) = self
            .browser
            .selected()
            .cloned()
            .or_else(|| selection.first().cloned())
        else {
            return;
        };
        let entries = match selection.len() > 1 {
            true => selection,
            false => vec![entry.clone()],
        };
        let at = entries
            .iter()
            .position(|shown| shown.path == entry.path)
            .unwrap_or(0);
        let subjects = entries
            .into_iter()
            .map(|entry| self.subject(entry))
            .collect();
        link.borrow_mut().open(subjects, at, full_screen);
        self.requests.push(Request::ShowQuickLook);
    }

    pub(crate) fn follow_selection_in_quick_look(&mut self, selected: Option<&Entry>) {
        let Some(link) = self.quick_look.clone() else {
            return;
        };
        let Some(shown) = link.borrow().follows().cloned() else {
            return;
        };
        match selected {
            Some(entry) if entry.path == shown => {}
            Some(entry) => {
                let subject = self.subject(entry.clone());
                link.borrow_mut().follow(subject);
            }
            None => link.borrow_mut().close(),
        }
    }

    pub(crate) fn toggle_quick_look_full_screen(&mut self) {
        match (&self.quick_look, self.quick_look_shown()) {
            (Some(link), true) => link
                .borrow_mut()
                .key(Key::Function(11), Modifiers::default()),
            _ => self.open_quick_look(true),
        }
    }

    pub(crate) fn quick_look_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let Some(link) = &self.quick_look else {
            return false;
        };
        let taken = link.borrow().takes_key(key, modifiers);
        if taken {
            link.borrow_mut().key(key, modifiers);
        }
        taken
    }
}
