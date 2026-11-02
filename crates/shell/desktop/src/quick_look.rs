use std::cell::RefCell;
use std::os::fd::BorrowedFd;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{NodeId, TreeId, TreeInfo, TreeUpdate};
use bande_quick_look::{Action, QuickLook, Subject};
use bande_toolkit::ui::{Event, Key, Modifiers, frame_time};
use bande_toolkit::{
    App, Color, Edges, KeyboardFocus, LayerKind, LayerOptions, Palette, Rect, Role, Scene,
    TextSystem, Transfer, Window, WindowEvent,
};
use bande_xdg::{file_uri, launch_with};

const NAMESPACE: &str = "quick-look";
const ROOT: NodeId = NodeId(1);

pub enum Message {
    Open {
        subjects: Vec<Subject>,
        at: usize,
        full_screen: bool,
    },
    Follow(Box<Subject>),
    Close,
    Key(Key, Modifiers),
}

#[derive(Default)]
pub struct Link {
    messages: Vec<Message>,
    open: bool,
    shown: bool,
    multiple: bool,
    path: Option<PathBuf>,
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

    pub fn is_multiple(&self) -> bool {
        self.shown && self.multiple
    }

    pub fn open(&mut self, subjects: Vec<Subject>, at: usize, full_screen: bool) {
        self.multiple = subjects.len() > 1;
        self.path = subjects.get(at).map(|subject| subject.entry.path.clone());
        self.open = true;
        self.shown = true;
        self.messages.push(Message::Open {
            subjects,
            at,
            full_screen,
        });
    }

    pub fn follow(&mut self, subject: Subject) {
        self.path = Some(subject.entry.path.clone());
        self.messages.push(Message::Follow(Box::new(subject)));
    }

    pub fn close(&mut self) {
        self.shown = false;
        self.messages.push(Message::Close);
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        self.messages.push(Message::Key(key, modifiers));
    }

    pub fn abandon(&mut self) {
        *self = Self::default();
    }

    pub fn has_messages(&self) -> bool {
        !self.messages.is_empty()
    }
}

pub fn quick_look_role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Overlay,
        anchor: Edges::ALL,
        exclusive_zone: -1,
        keyboard: KeyboardFocus::None,
        namespace: NAMESPACE.to_owned(),
    })
}

pub struct QuickLookSurface {
    link: QuickLookLink,
    shown: Option<QuickLook>,
    text: TextSystem,
    palette: Palette,
    reduce_motion: bool,
}

impl QuickLookSurface {
    pub fn new(window: &Window, text: TextSystem, link: QuickLookLink) -> Self {
        let appearance = window.appearance();
        let mut surface = Self {
            link,
            shown: None,
            text,
            palette: appearance.palette(),
            reduce_motion: appearance.reduce_motion,
        };
        surface.apply(Instant::now());
        surface
    }

    fn apply(&mut self, now: Instant) -> bool {
        let messages = std::mem::take(&mut self.link.borrow_mut().messages);
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
                ) => {
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
                    None,
                ) => match QuickLook::open(subjects, at, full_screen, self.reduce_motion, now) {
                    Ok(shown) => self.shown = Some(shown),
                    Err(error) => {
                        tracing::warn!("cannot start Quick Look: {error}");
                        self.link.borrow_mut().abandon();
                    }
                },
                (Message::Follow(subject), Some(shown)) => shown.follow(*subject),
                (Message::Close, Some(shown)) => shown.close(now),
                (Message::Key(key, modifiers), Some(shown)) => {
                    shown.key(key, modifiers, now);
                    if shown.is_closing() {
                        self.link.borrow_mut().shown = false;
                    }
                }
                _ => {}
            }
        }
        changed
    }

    fn act(&mut self, window: &mut Window, action: Action) {
        match action {
            Action::Open { app, path } => {
                let token = window.activation_token(app.app_id()).ok();
                if let Err(error) = launch_with(&app, &[path], token.as_deref()) {
                    tracing::warn!("cannot open with {}: {error}", app.id);
                }
            }
            Action::Copy(path) => window.copy_transfer(Transfer::uris(vec![file_uri(&path)])),
            Action::Close => self.link.borrow_mut().shown = false,
        }
    }
}

impl App for QuickLookSurface {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let now = frame_time().presentation;
        self.apply(now);
        let (width, height) = window.size();
        let mut items = Vec::new();
        if let Some(shown) = &mut self.shown {
            if shown.is_closed(now) {
                self.shown = None;
                self.link.borrow_mut().open = false;
            } else {
                let bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
                let scale = window.scale() as f32;
                items = shown.paint(&mut self.text, self.palette, bounds, scale, now);
                window.set_input_region(Some(&shown.input_region()));
            }
        }
        Scene {
            width,
            height,
            background: Color::TRANSPARENT,
            items,
        }
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        match event {
            WindowEvent::Input(input) => {
                let now = frame_time().on_screen();
                let Some(shown) = &mut self.shown else {
                    return;
                };
                let (handled, action) = shown.handle(&input, now);
                if let Event::PointerMove(pointer) | Event::PointerEnter(pointer) = &input
                    && let Some(cursor) = shown.cursor(pointer.position)
                {
                    window.set_cursor(cursor);
                }
                if let Some(action) = action {
                    self.act(window, action);
                }
                if handled {
                    window.request_redraw();
                }
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.palette = appearance.palette();
                self.reduce_motion = appearance.reduce_motion;
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        self.shown
            .as_ref()
            .map(QuickLook::sources)
            .unwrap_or_default()
    }

    fn source_ready(&mut self, window: &mut Window, index: usize) {
        if self
            .shown
            .as_mut()
            .is_some_and(|shown| shown.source_ready(index))
        {
            window.request_redraw();
        }
    }

    fn deadline(&self) -> Option<Instant> {
        let now = self.link.borrow().has_messages().then(Instant::now);
        [now, self.shown.as_ref().and_then(QuickLook::deadline)]
            .into_iter()
            .flatten()
            .min()
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        let applied = self.apply(now);
        let ticked = self.shown.as_mut().is_some_and(|shown| shown.tick(now));
        if applied || ticked {
            window.request_redraw();
        }
    }

    fn animating(&self) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|shown| shown.animating(frame_time().presentation))
    }

    fn should_close(&mut self, _window: &mut Window) -> bool {
        false
    }

    fn accessibility(&self) -> Option<TreeUpdate> {
        let node = self.shown.as_ref()?.access();
        Some(TreeUpdate {
            nodes: vec![(ROOT, node)],
            tree: Some(TreeInfo::new(ROOT)),
            tree_id: TreeId::ROOT,
            focus: ROOT,
        })
    }
}
