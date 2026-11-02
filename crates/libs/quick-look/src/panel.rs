use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{ActionRequest, TreeUpdate};
use bande_toolkit::ui::{Cursor, Event, Key, Modifiers};
use bande_toolkit::{Appearance, Item, Palette, Point, Rect, TextSystem};

use crate::{Action, QuickLook, Subject};

enum Message {
    Open {
        subjects: Vec<Subject>,
        at: usize,
        full_screen: bool,
    },
    Follow(Box<Subject>),
    Close,
    ToggleFullScreen,
    Key(Key, Modifiers),
}

#[derive(Default)]
pub struct Link {
    messages: Vec<Message>,
    posted: Option<Instant>,
    open: bool,
    shown: bool,
    multiple: bool,
    typing: bool,
    path: Option<PathBuf>,
    origin: Option<Rect>,
    copied: Option<PathBuf>,
}

pub type QuickLookLink = Rc<RefCell<Link>>;

impl Link {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_shown(&self) -> bool {
        self.shown
    }

    pub fn follows(&self) -> Option<&PathBuf> {
        self.path.as_ref().filter(|_| self.shown && !self.multiple)
    }

    pub fn open(&mut self, subjects: Vec<Subject>, at: usize, full_screen: bool) {
        self.multiple = subjects.len() > 1;
        self.path = subjects.get(at).map(|subject| subject.entry.path.clone());
        self.origin = subjects.get(at).and_then(|subject| subject.origin);
        self.open = true;
        self.shown = true;
        self.posted.get_or_insert_with(Instant::now);
        self.messages.push(Message::Open {
            subjects,
            at,
            full_screen,
        });
    }

    pub fn follow(&mut self, subject: Subject) {
        self.path = Some(subject.entry.path.clone());
        self.origin = subject.origin;
        self.posted.get_or_insert_with(Instant::now);
        self.messages.push(Message::Follow(Box::new(subject)));
    }

    pub fn close(&mut self) {
        self.shown = false;
        self.posted.get_or_insert_with(Instant::now);
        self.messages.push(Message::Close);
    }

    pub fn copy(&mut self, path: PathBuf) {
        self.copied = Some(path);
    }

    pub fn take_copied(&mut self) -> Option<PathBuf> {
        self.copied.take()
    }

    pub fn take_origin(&mut self) -> Option<Rect> {
        self.origin.take()
    }

    pub fn takes_key(&self, key: Key, modifiers: Modifiers) -> bool {
        if !self.shown || modifiers.logo {
            return false;
        }
        let command = modifiers.control;
        let plain = !command && !modifiers.alt && !modifiers.shift;
        if self.typing
            && matches!(
                key,
                Key::Character(_) | Key::Space | Key::Backspace | Key::Enter
            )
            && !command
        {
            return true;
        }
        match key {
            Key::Escape => true,
            Key::PageUp | Key::PageDown | Key::Home | Key::End => plain,
            Key::Left | Key::Right => plain && self.multiple,
            Key::Enter => command,
            Key::Character('=' | '+' | '-' | '0' | 'l' | 'L') => command,
            _ => false,
        }
    }

    pub fn toggle_full_screen(&mut self) {
        self.posted.get_or_insert_with(Instant::now);
        self.messages.push(Message::ToggleFullScreen);
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.posted.get_or_insert_with(Instant::now);
        self.messages.push(Message::Key(key, modifiers));
    }

    fn posted(&self) -> Option<Instant> {
        self.posted
    }

    fn take_messages(&mut self) -> Vec<Message> {
        self.posted = None;
        std::mem::take(&mut self.messages)
    }

    pub fn abandon(&mut self) {
        *self = Self::default();
    }
}

pub struct Panel {
    link: QuickLookLink,
    shown: Option<QuickLook>,
    text: TextSystem,
    palette: Palette,
    reduce_motion: bool,
    windowed: bool,
    finished: bool,
}

impl Panel {
    pub fn new(
        link: QuickLookLink,
        text: TextSystem,
        appearance: Appearance,
        windowed: bool,
    ) -> Self {
        let mut panel = Self {
            link,
            shown: None,
            text,
            palette: appearance.palette(),
            reduce_motion: appearance.reduce_motion,
            windowed,
            finished: false,
        };
        panel.apply(Instant::now());
        panel
    }

    pub fn link(&self) -> &QuickLookLink {
        &self.link
    }

    pub fn set_appearance(&mut self, appearance: Appearance) {
        self.palette = appearance.palette();
        self.reduce_motion = appearance.reduce_motion;
    }

    fn sync(&mut self) {
        let mut link = self.link.borrow_mut();
        link.typing = self.shown.as_ref().is_some_and(QuickLook::wants_typing);
        if self.shown.as_ref().is_some_and(QuickLook::is_closing) {
            link.shown = false;
        }
    }

    pub fn apply(&mut self, now: Instant) -> bool {
        if self.finished {
            return false;
        }
        let messages = self.link.borrow_mut().take_messages();
        let changed = !messages.is_empty();
        for message in messages {
            match (message, &mut self.shown) {
                (
                    Message::Open {
                        subjects,
                        at,
                        full_screen,
                    },
                    Some(shown),
                ) if !shown.is_closing() => {
                    shown.show(subjects, at);
                    if shown.is_full_screen() != full_screen {
                        shown.toggle_full_screen();
                    }
                }
                (
                    Message::Open {
                        subjects,
                        at,
                        full_screen,
                    },
                    _,
                ) => {
                    let opened = QuickLook::open(
                        subjects,
                        at,
                        full_screen,
                        self.reduce_motion,
                        self.windowed,
                        now,
                    );
                    match opened {
                        Ok(shown) => self.shown = Some(shown),
                        Err(error) => {
                            tracing::warn!("cannot start Quick Look: {error}");
                            self.link.borrow_mut().abandon();
                        }
                    }
                }
                (Message::Follow(subject), Some(shown)) => shown.follow(*subject),
                (Message::Close, Some(shown)) => shown.close(now),
                (Message::ToggleFullScreen, Some(shown)) => shown.toggle_full_screen(),
                (Message::Key(key, modifiers), Some(shown)) => {
                    shown.key(key, modifiers, now);
                }
                _ => {}
            }
        }
        self.sync();
        changed
    }

    pub fn paint(&mut self, bounds: Rect, scale: f32, now: Instant) -> Option<Vec<Item>> {
        self.apply(now);
        let shown = self.shown.as_mut()?;
        if shown.is_closed(now) {
            self.shown = None;
            self.finished = self.windowed;
            self.link.borrow_mut().open = false;
            return None;
        }
        shown.resized_window((bounds.width, bounds.height));
        Some(shown.paint(&mut self.text, self.palette, bounds, scale, now))
    }

    pub fn is_closed(&self) -> bool {
        self.shown.is_none()
    }

    pub fn is_full_screen(&self) -> bool {
        self.shown.as_ref().is_some_and(QuickLook::is_full_screen)
    }

    pub fn size(&self) -> Option<(f32, f32)> {
        self.shown.as_ref().map(QuickLook::size)
    }

    pub fn input_region(&self) -> Vec<Rect> {
        self.shown
            .as_ref()
            .map(QuickLook::input_region)
            .unwrap_or_default()
    }

    pub fn handle(&mut self, event: &Event, now: Instant) -> bool {
        let handled = self
            .shown
            .as_mut()
            .is_some_and(|shown| shown.handle(event, now));
        self.sync();
        handled
    }

    pub fn cursor(&self, point: Point) -> Option<Cursor> {
        self.shown.as_ref()?.cursor(point)
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        self.shown
            .as_mut()
            .map(QuickLook::take_actions)
            .unwrap_or_default()
    }

    pub fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.shown
            .as_ref()
            .map(QuickLook::sources)
            .unwrap_or_default()
    }

    pub fn source_ready(&mut self, index: usize) -> bool {
        let changed = self
            .shown
            .as_mut()
            .is_some_and(|shown| shown.source_ready(index));
        self.sync();
        changed
    }

    pub fn deadline(&self) -> Option<Instant> {
        let now = self.link.borrow().posted().filter(|_| !self.finished);
        [now, self.shown.as_ref().and_then(QuickLook::deadline)]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let applied = self.apply(now);
        let ticked = self.shown.as_mut().is_some_and(|shown| shown.tick(now));
        applied || ticked
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|shown| shown.animating(now))
    }

    pub fn accessibility(&self) -> Option<TreeUpdate> {
        self.shown.as_ref().map(QuickLook::accessibility)
    }

    pub fn perform(&mut self, request: &ActionRequest, now: Instant) -> bool {
        let performed = self
            .shown
            .as_mut()
            .is_some_and(|shown| shown.perform(request, now));
        self.sync();
        performed
    }
}
