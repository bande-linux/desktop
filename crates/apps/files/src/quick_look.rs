use std::time::Instant;

use bande_fs::Entry;
use bande_quick_look::{Action, QuickLook, Subject};
use bande_thumbnails::ThumbnailSize;
use bande_toolkit::ui::{Event, Key, Modifiers};
use bande_toolkit::{Item, Rect};

use crate::L10N;
use crate::view::{FilesView, Hit, Request};

const ICON_PIXELS: u32 = 256;
const FOLDER_MIME: &str = "inode/directory";

impl FilesView {
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
        let icon = self
            .thumbs
            .get(&entry, ThumbnailSize::XLarge)
            .or_else(|| self.entry_icon(&entry, ICON_PIXELS));
        let origin = self.origin_of(&entry);
        Subject {
            entry,
            mime,
            kind,
            app,
            icon,
            origin,
        }
    }

    fn now(&mut self) -> Instant {
        self.ui.context().now()
    }

    pub(crate) fn toggle_quick_look(&mut self) {
        let now = self.now();
        match &mut self.quick_look {
            Some(shown) if !shown.is_closing() => shown.close(now),
            _ => self.open_quick_look(false),
        }
    }

    pub(crate) fn open_quick_look(&mut self, full_screen: bool) {
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
        let now = self.now();
        let reduce_motion = self.ui.context().reduce_motion;
        match QuickLook::open(subjects, at, full_screen, reduce_motion, now) {
            Ok(shown) => self.quick_look = Some(shown),
            Err(error) => tracing::warn!("cannot start Quick Look: {error}"),
        }
    }

    pub(crate) fn follow_selection_in_quick_look(&mut self, selected: Option<&Entry>) {
        let Some(shown) = &self.quick_look else {
            return;
        };
        if !shown.follows_selection() || selected.map(|entry| &entry.path) == shown.path() {
            return;
        }
        match selected {
            Some(entry) => {
                let subject = self.subject(entry.clone());
                if let Some(shown) = &mut self.quick_look {
                    shown.follow(subject);
                }
            }
            None => {
                let now = self.now();
                if let Some(shown) = &mut self.quick_look {
                    shown.close(now);
                }
            }
        }
    }

    pub(crate) fn toggle_quick_look_full_screen(&mut self) {
        match &mut self.quick_look {
            Some(shown) if !shown.is_closing() => shown.toggle_full_screen(),
            _ => self.open_quick_look(true),
        }
    }

    pub(crate) fn quick_look_key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        let now = self.now();
        self.quick_look
            .as_mut()
            .is_some_and(|shown| !shown.is_closing() && shown.key(key, modifiers, now))
    }

    pub(crate) fn quick_look_event(&mut self, event: &Event) -> bool {
        let now = self.now();
        let Some(shown) = &mut self.quick_look else {
            return false;
        };
        let position = match event {
            Event::PointerDown(pointer) => Some(pointer.position),
            Event::Scroll(scroll) => Some(scroll.position),
            _ => None,
        };
        if position.is_some_and(|at| !shown.contains(at)) {
            return false;
        }
        let (handled, action) = shown.handle(event, now);
        match action {
            Some(Action::Open { app, path }) => self.requests.push(Request::Open {
                app,
                files: vec![path],
            }),
            Some(Action::Copy(path)) => self.requests.push(Request::CopyFiles {
                paths: vec![path],
                cut: false,
            }),
            Some(Action::Close) | None => {}
        }
        handled || position.is_some()
    }

    pub(crate) fn quick_look_bounds(&self) -> Rect {
        match self
            .quick_look
            .as_ref()
            .is_some_and(QuickLook::is_full_screen)
        {
            true => Rect::new(0.0, 0.0, self.size.0, self.size.1),
            false => self.sheet_frame(),
        }
    }

    pub(crate) fn paint_quick_look(&mut self, items: &mut Vec<Item>) {
        let bounds = self.quick_look_bounds();
        let cx = self.ui.context();
        let now = cx.now();
        let Some(shown) = &mut self.quick_look else {
            return;
        };
        if shown.is_closed(now) {
            self.quick_look = None;
            return;
        }
        let scale = cx.display_scale as f32;
        items.extend(shown.paint(&mut cx.text, cx.palette, bounds, scale, now));
    }
}
