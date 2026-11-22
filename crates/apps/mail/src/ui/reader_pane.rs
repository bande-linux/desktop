use std::io::{self, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use accesskit::{Node, Role};
use bande_datetime::DateFormats;
use bande_l10n::tr;
use bande_toolkit::ui::{
    Alert, Context, Key, Menu, MenuItem, Pointer, PointerButton, ScrollState, focus_ring_for,
};
use bande_toolkit::{Item, Palette, Point, Radius, Rect, Shape, TextRole, TextStyle};
use jiff::tz::TimeZone;

use crate::message::Address;
use crate::net::Network;
use crate::picture::Picture;
use crate::store::{MessageId, Summary};
use crate::ui::access::clickable;
use crate::ui::format::full_date;
use crate::ui::reader::{
    Clock, Layout, Reading, Remote, Respond, Target, Thread, focus_bar, lay_out, link_host,
};
use crate::ui::symbols::Symbols;

type Spot = (usize, usize);

const MAX_PICTURE_BYTES: usize = 10 << 20;
const LINK_TARGET_INSET: f32 = 8.0;
const LINK_TARGET_PADDING: f32 = 8.0;
const PAGE_OVERLAP: f32 = 40.0;
const READER_LINE: f32 = 40.0;

#[derive(Default)]
pub(crate) struct ReaderPane {
    pub(crate) thread: Option<Thread>,
    pub(crate) layout: Option<(f32, Layout)>,
    pub(crate) selection: Option<(Spot, Spot)>,
    pub(crate) selecting: bool,
    pub(crate) target: Option<(usize, Target)>,
    pub(crate) scroll: ScrollState,
    opened: Vec<((PathBuf, usize), PathBuf, SystemTime)>,
}

pub(crate) struct Look<'a> {
    pub(crate) palette: &'a Palette,
    pub(crate) symbols: &'a mut Symbols,
    pub(crate) clock: Clock<'a>,
    pub(crate) focused: bool,
    pub(crate) sheet: bool,
}

impl ReaderPane {
    pub(crate) fn focused(&self) -> Option<(&Summary, &Reading)> {
        self.thread.as_ref().and_then(Thread::focused)
    }

    pub(crate) fn printable(
        &self,
        zone: &TimeZone,
        formats: &DateFormats,
    ) -> Option<(String, String)> {
        let (summary, reading) = self.focused()?;
        let envelope = &reading.content.envelope;
        let names = |list: &[crate::message::Address]| {
            list.iter()
                .map(crate::message::Address::full)
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut lines = vec![envelope.subject.clone(), names(&envelope.from)];
        lines.extend(envelope.date.map(|date| full_date(date, zone, formats)));
        lines.push(tr!("To: {names}", names = names(&envelope.to)));
        if !envelope.cc.is_empty() {
            lines.push(tr!("Cc: {names}", names = names(&envelope.cc)));
        }
        let text = format!("{}\n\n{}", lines.join("\n"), reading.content.text());
        let title = match summary.subject.is_empty() {
            true => tr!("No Subject"),
            false => summary.subject.clone(),
        };
        Some((title, text))
    }

    fn reading_mut(&mut self, id: MessageId) -> Option<&mut Reading> {
        self.thread
            .as_mut()
            .and_then(|thread| thread.entry_mut(id))
            .and_then(|entry| entry.reading.as_mut())
    }

    pub(crate) fn focus_entry(&mut self, id: MessageId) {
        if let Some(thread) = self.thread.as_mut()
            && thread.focus != id
        {
            thread.focus = id;
            self.layout = None;
        }
    }

    pub(crate) fn toggle_quotes(&mut self, id: MessageId) {
        self.focus_entry(id);
        if let Some(entry) = self.thread.as_mut().and_then(|thread| thread.entry_mut(id)) {
            entry.quotes = !entry.quotes;
            self.layout = None;
        }
    }

    pub(crate) fn paint(&mut self, cx: &mut Context, look: Look<'_>, frame: Rect) -> Vec<Item> {
        let mut items = Vec::new();
        let Some(thread) = &self.thread else {
            return items;
        };
        let palette = *look.palette;
        if self
            .layout
            .as_ref()
            .is_none_or(|(width, _)| *width != frame.width)
        {
            let layout = lay_out(
                thread,
                frame.width,
                &mut cx.text,
                &palette,
                look.symbols,
                look.clock,
            );
            self.target = self.target.take().and_then(|(index, target)| {
                let found = |at: &usize| {
                    layout
                        .targets
                        .get(*at)
                        .is_some_and(|(_, kept)| *kept == target)
                };
                Some(index)
                    .filter(found)
                    .or_else(|| layout.targets.iter().position(|(_, kept)| *kept == target))
                    .map(|index| (index, target))
            });
            self.layout = Some((frame.width, layout));
            self.selection = None;
        }
        let Some((_, layout)) = &self.layout else {
            return items;
        };
        let reveal = self
            .thread
            .as_mut()
            .filter(|thread| thread.reveal)
            .and_then(|thread| {
                thread.reveal = false;
                layout
                    .tops
                    .iter()
                    .find(|(id, _)| *id == thread.focus)
                    .map(|(_, top)| *top)
            });
        self.scroll.set_extent(layout.height, frame.height);
        if let Some(top) = reveal {
            self.scroll.follow(cx, top, top + frame.height);
        }
        let origin = Point::new(frame.x, frame.y - self.scroll.offset(cx));
        let active = look.focused && !look.sheet && cx.window_active;
        let bar = match active {
            true => palette.accent,
            false => palette.ink3,
        };
        let mut ring = Vec::new();
        if let Some((index, target)) = &self.target
            && active
        {
            for (rect, _) in layout.targets[*index..]
                .iter()
                .take_while(|(_, shown)| shown == target)
            {
                focus_ring_for(&palette, Shape::rounded(*rect, Radius::XS), &mut ring);
            }
        }
        let selected = match look.focused {
            true => palette.accent_soft,
            false => palette.well,
        };
        let highlight =
            self.selected_range()
                .map(|(start, end)| {
                    (start.0..=end.0)
                        .filter_map(|index| {
                            let (_, top, paragraph) = layout.texts.get(index)?;
                            let from = if index == start.0 { start.1 } else { 0 };
                            let to = if index == end.0 {
                                end.1
                            } else {
                                paragraph.text().len()
                            };
                            Some(paragraph.span_rects(from..to).into_iter().map(|rect| {
                                Item::Fill {
                                    shape: Shape::rect(Rect::new(
                                        rect.x + top.x,
                                        rect.y + top.y,
                                        rect.width,
                                        rect.height,
                                    )),
                                    paint: selected.into(),
                                }
                            }))
                        })
                        .flatten()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
        items.push(Item::Clip {
            shape: Shape::rect(frame),
            items: vec![Item::Translate {
                offset: origin,
                items: highlight
                    .into_iter()
                    .chain(layout.items.iter().cloned())
                    .chain(
                        self.thread
                            .as_ref()
                            .and_then(|thread| focus_bar(thread, layout, bar)),
                    )
                    .chain(ring)
                    .collect(),
            }],
        });
        self.scroll.paint_scrollbar(cx, frame, &mut items);
        items
    }

    pub(crate) fn visible_targets(&self, cx: &Context, frame: Rect) -> Vec<(Rect, Target)> {
        let Some((_, layout)) = &self.layout else {
            return Vec::new();
        };
        let origin = self.origin(cx, frame);
        layout
            .targets
            .iter()
            .map(|(rect, target)| {
                (
                    Rect::new(
                        rect.x + origin.x,
                        rect.y + origin.y,
                        rect.width,
                        rect.height,
                    ),
                    target.clone(),
                )
            })
            .filter(|(rect, _)| rect.y + rect.height > frame.y && rect.y < frame.y + frame.height)
            .collect()
    }

    pub(crate) fn scroll_key(&mut self, cx: &mut Context, frame: Rect, key: Key) -> bool {
        let page = frame.height - PAGE_OVERLAP;
        let line = (READER_LINE * cx.text.scale()).round();
        let offset = self.scroll.offset(cx);
        let target = match key {
            Key::Up => offset - line,
            Key::Down => offset + line,
            Key::PageUp => offset - page,
            Key::PageDown => offset + page,
            Key::Home => 0.0,
            _ => f32::MAX,
        };
        self.scroll.scroll_to(cx, target)
    }

    fn origin(&self, cx: &Context, frame: Rect) -> Point {
        Point::new(frame.x, frame.y - self.scroll.offset(cx))
    }

    fn spot_at(&self, cx: &Context, frame: Rect, at: Point) -> Option<Spot> {
        let origin = self.origin(cx, frame);
        let (x, y) = (at.x - origin.x, at.y - origin.y);
        let (_, layout) = self.layout.as_ref()?;
        let outside = |point: f32, start: f32, length: f32| {
            (start - point).max(point - start - length).max(0.0)
        };
        let distance = |(_, top, paragraph): &(MessageId, Point, bande_toolkit::Paragraph)| {
            (
                outside(y, top.y, paragraph.height()),
                outside(x, top.x, paragraph.width()),
            )
        };
        let (index, (_, top, paragraph)) = layout.texts.iter().enumerate().min_by(|a, b| {
            let (a, b) = (distance(a.1), distance(b.1));
            a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
        })?;
        Some((index, paragraph.byte_at(x - top.x, y - top.y)))
    }

    fn word_at(&self, (index, byte): Spot) -> (Spot, Spot) {
        let Some(text) = self
            .layout
            .as_ref()
            .and_then(|(_, layout)| layout.texts.get(index))
            .map(|(_, _, paragraph)| paragraph.text())
        else {
            return ((index, byte), (index, byte));
        };
        let word = |character: char| character.is_alphanumeric() || character == '_';
        let before = text.get(..byte).unwrap_or_default();
        let after = text.get(byte..).unwrap_or_default();
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, character)| word(*character))
            .last()
            .map_or(byte, |(at, _)| at);
        let end = after
            .char_indices()
            .find(|(_, character)| !word(*character))
            .map_or(text.len(), |(at, _)| byte + at);
        ((index, start), (index, end))
    }

    pub(crate) fn start_selection(&mut self, cx: &Context, frame: Rect, pointer: &Pointer) {
        self.selection =
            self.spot_at(cx, frame, pointer.position)
                .map(|spot| match pointer.count {
                    2 => self.word_at(spot),
                    _ => (spot, spot),
                });
        self.selecting = self.selection.is_some();
    }

    pub(crate) fn extend_selection(&mut self, cx: &Context, frame: Rect, at: Point) -> bool {
        let Some(spot) = self.spot_at(cx, frame, at) else {
            return false;
        };
        match &mut self.selection {
            Some((_, focus)) if *focus != spot => {
                *focus = spot;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn finish_selection(&mut self) -> Option<String> {
        self.selecting = false;
        self.selected_text()
    }

    fn selected_range(&self) -> Option<(Spot, Spot)> {
        let (anchor, focus) = self.selection?;
        let (start, end) = match anchor <= focus {
            true => (anchor, focus),
            false => (focus, anchor),
        };
        (start != end).then_some((start, end))
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        let (_, layout) = self.layout.as_ref()?;
        let pieces = (start.0..=end.0)
            .filter_map(|index| {
                let text = layout.texts.get(index)?.2.text();
                let from = if index == start.0 { start.1 } else { 0 };
                let to = if index == end.0 { end.1 } else { text.len() };
                text.get(from..to)
            })
            .collect::<Vec<_>>();
        Some(pieces.join("\n"))
    }

    pub(crate) fn select_entry_text(&mut self) {
        let Some(focus) = self.thread.as_ref().map(|thread| thread.focus) else {
            return;
        };
        let Some((_, layout)) = &self.layout else {
            return;
        };
        let mut texts = layout
            .texts
            .iter()
            .enumerate()
            .filter(|(_, (id, _, _))| *id == focus);
        let Some(first) = texts.next() else {
            return;
        };
        let (last, (_, _, paragraph)) = texts.next_back().unwrap_or(first);
        let first = first.0;
        self.selection = Some(((first, 0), (last, paragraph.text().len())));
    }

    pub(crate) fn text_menu(&self) -> Menu {
        Menu::new(vec![
            MenuItem::new(&tr!("Copy"), "copy-selection").enabled(self.selected_range().is_some()),
            MenuItem::new(&tr!("Select All"), "select-text"),
        ])
    }

    pub(crate) fn stops(&self) -> Vec<usize> {
        let Some((_, layout)) = &self.layout else {
            return Vec::new();
        };
        let targets = &layout.targets;
        (0..targets.len())
            .filter(|index| {
                let target = &targets[*index].1;
                !matches!(target, Target::Card(_))
                    && index
                        .checked_sub(1)
                        .is_none_or(|before| targets[before].1 != *target)
            })
            .collect()
    }

    pub(crate) fn set_target(&mut self, cx: &Context, index: usize) {
        let Some((rect, target)) = self
            .layout
            .as_ref()
            .and_then(|(_, layout)| layout.targets.get(index).cloned())
        else {
            return;
        };
        self.target = Some((index, target));
        self.scroll.reveal(cx, rect.y, rect.y + rect.height);
    }

    pub(crate) fn step_target(&mut self, cx: &Context, forward: bool) -> bool {
        let stops = self.stops();
        let at = self
            .target
            .as_ref()
            .and_then(|(index, _)| stops.iter().position(|stop| stop == index));
        let next = match (at, forward) {
            (None, true) => stops.first().copied(),
            (None, false) => None,
            (Some(at), true) => stops.get(at + 1).copied(),
            (Some(at), false) => at.checked_sub(1).map(|before| stops[before]),
        };
        self.target = None;
        match next {
            Some(index) => {
                self.set_target(cx, index);
                true
            }
            None => at.is_some() && !forward,
        }
    }

    pub(crate) fn pressed_target(
        &self,
        cx: &Context,
        frame: Rect,
        secondary: bool,
    ) -> Option<(Target, Pointer)> {
        let (index, target) = self.target.clone()?;
        let has_menu = matches!(
            target,
            Target::Link(..) | Target::Address(_) | Target::Attachment(..)
        );
        let rect = self
            .layout
            .as_ref()
            .and_then(|(_, layout)| layout.targets.get(index))
            .map(|(rect, _)| *rect)
            .filter(|_| has_menu || !secondary)?;
        let origin = self.origin(cx, frame);
        let at = Point::new(
            origin.x + rect.x + rect.width / 2.0,
            origin.y + rect.y + rect.height,
        );
        let button = match secondary {
            true => PointerButton::Secondary,
            false => PointerButton::Primary,
        };
        Some((target, Pointer::at(at).with_button(button)))
    }

    pub(crate) fn claimed_host(&self, url: &str) -> Option<String> {
        let (_, layout) = self.layout.as_ref()?;
        layout.targets.iter().find_map(|(_, target)| match target {
            Target::Link(known, claimed, _) if known == url => claimed.clone(),
            _ => None,
        })
    }

    pub(crate) fn attachment(&self, index: usize) -> Option<(String, Vec<u8>)> {
        let attachment = self.focused()?.1.content.every_attachment().nth(index)?;
        let name = attachment
            .name
            .replace(['/', '\\', '\0'], "_")
            .trim_start_matches('.')
            .to_owned();
        let name = match name.is_empty() {
            true => "attachment".to_owned(),
            false => name,
        };
        Some((name, attachment.data.clone()))
    }

    pub(crate) fn open_attachment(&mut self, folder: PathBuf, index: usize) -> Option<PathBuf> {
        let (name, data) = self.attachment(index)?;
        let key = (folder.clone(), index);
        let ours = self
            .opened
            .iter()
            .find(|(known, ..)| *known == key)
            .map(|(_, path, _)| path.clone());
        let path = folder.join(&name);
        let kept = ours.or_else(|| {
            std::fs::read(&path)
                .is_ok_and(|kept| kept == data)
                .then_some(path)
        });
        if kept.is_some() {
            return kept;
        }
        let written = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&folder)
            .and_then(|()| write_new(&folder, &name, &data));
        match written {
            Ok(path) => {
                if let Ok(modified) =
                    std::fs::metadata(&path).and_then(|metadata| metadata.modified())
                {
                    self.opened.push((key, path.clone(), modified));
                }
                Some(path)
            }
            Err(error) => {
                tracing::warn!("cannot open {name}: {error}");
                None
            }
        }
    }

    pub(crate) fn save_to_downloads(&self, index: usize) -> Result<(), (String, io::Error)> {
        let Some((name, data)) = self.attachment(index) else {
            return Ok(());
        };
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Err((
                name,
                io::Error::new(
                    io::ErrorKind::NotFound,
                    tr!("There is no Downloads folder, because HOME is not set."),
                ),
            ));
        };
        let user_dirs = bande_xdg::config_home()
            .and_then(|config| std::fs::read_to_string(config.join("user-dirs.dirs")).ok());
        let folder =
            bande_xdg::user_dir(&home, user_dirs.as_deref(), bande_xdg::UserDir::Downloads);
        std::fs::create_dir_all(&folder)
            .and_then(|()| write_new(&folder, &name, &data))
            .map(|_| ())
            .map_err(|error| {
                tracing::warn!("cannot save {name}: {error}");
                (name, error)
            })
    }

    pub(crate) fn remove_unedited_attachments(&mut self) {
        let unedited = self
            .opened
            .drain(..)
            .filter(|(_, path, written)| {
                std::fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .is_ok_and(|modified| modified == *written)
            })
            .map(|(_, path, _)| path)
            .collect::<Vec<_>>();
        for path in unedited {
            if std::fs::remove_file(&path).is_ok()
                && let Some(folder) = path.parent()
            {
                let _ = std::fs::remove_dir(folder);
            }
        }
    }

    pub(crate) fn start_remote(&mut self, id: MessageId) -> Option<Vec<String>> {
        let reading = self.reading_mut(id)?;
        if reading.remote != Remote::Blocked {
            return None;
        }
        reading.remote = Remote::Loading;
        let sources = reading.remote_sources();
        self.layout = None;
        Some(sources)
    }

    pub(crate) fn add_pictures(&mut self, id: MessageId, pictures: Vec<(String, Picture)>) {
        if let Some(reading) = self.reading_mut(id) {
            for (source, picture) in pictures {
                reading.add_picture(source, picture);
            }
            reading.remote = Remote::Loaded;
            self.layout = None;
        }
    }

    fn target_label(&self, target: &Target) -> Option<(Role, String)> {
        let thread = self.thread.as_ref()?;
        Some(match target {
            Target::Card(_) => return None,
            Target::Expand(id) => {
                let entry = thread.entry(*id)?;
                let sender = match entry.summary.sender.shown() {
                    "" => tr!("No Sender"),
                    shown => shown.to_owned(),
                };
                (
                    Role::Button,
                    tr!("Show Message from {sender}", sender = sender),
                )
            }
            Target::Quotes(id) => (
                Role::Button,
                match thread.entry(*id)?.quotes {
                    true => tr!("Hide Quoted Text"),
                    false => tr!("Show Quoted Text"),
                },
            ),
            Target::Respond(_, Respond::Reply) => (Role::Button, tr!("Reply")),
            Target::Respond(_, Respond::ReplyAll) => (Role::Button, tr!("Reply All")),
            Target::Respond(_, Respond::Forward) => (Role::Button, tr!("Forward")),
            Target::Link(_, _, shown) => (Role::Link, shown.clone()),
            Target::Address(address) => (Role::Button, address.full()),
            Target::Attachment(id, index) => {
                let attachment = thread
                    .entry(*id)?
                    .reading
                    .as_ref()?
                    .content
                    .every_attachment()
                    .nth(*index)?;
                (Role::Button, attachment.name.clone())
            }
            Target::LoadRemote(_) => (Role::Button, tr!("Load Remote Content")),
            Target::LookUpKey(_) => (Role::Button, tr!("Look Up Key")),
            Target::TrustIssuer(_) => (Role::Button, tr!("Trust Issuer…")),
            Target::Unseal(_) => (Role::Button, tr!("Try Again")),
        })
    }

    pub(crate) fn target_node(&self, target: &Target) -> Option<Node> {
        let (role, label) = self.target_label(target)?;
        let mut node = clickable(role, &label);
        if let Target::Link(url, ..) = target {
            node.set_url(url.as_str());
        }
        Some(node)
    }

    pub(crate) fn articles(
        &self,
        zone: &TimeZone,
        formats: &DateFormats,
    ) -> Vec<(usize, MessageId, Node, Node)> {
        let Some(thread) = &self.thread else {
            return Vec::new();
        };
        let mut articles = Vec::new();
        for (index, entry) in thread.entries.iter().enumerate() {
            let Some(reading) = &entry.reading else {
                continue;
            };
            let sender = match reading.content.envelope.sender().shown() {
                "" => tr!("No Sender"),
                shown => shown.to_owned(),
            };
            let date = reading
                .content
                .envelope
                .date
                .map(|date| full_date(date, zone, formats))
                .unwrap_or_default();
            let mut article = clickable(
                Role::Article,
                &tr!("From {sender}, {date}", sender = sender, date = date),
            );
            let notes =
                crate::ui::reader::security_notes(entry.summary.id, &reading.content.security)
                    .into_iter()
                    .map(|(note, _)| note)
                    .collect::<Vec<_>>();
            if !notes.is_empty() {
                article.set_description(notes.join(" "));
            }
            let mut text = Node::new(Role::Label);
            text.set_value(reading.document.text());
            articles.push((index, entry.summary.id, article, text));
        }
        articles
    }
}

pub(crate) fn link_menu(url: &str) -> Menu {
    Menu::new(vec![
        MenuItem::new(&tr!("Open Link"), &format!("open-link:{url}")),
        MenuItem::new(&tr!("Copy Link"), &format!("copy:{url}")),
    ])
}

pub(crate) fn address_menu(address: &Address) -> Menu {
    Menu::new(vec![
        MenuItem::new(
            &tr!("New Message"),
            &format!("compose-to:{}", address.full()),
        ),
        MenuItem::new(&tr!("Copy Address"), &format!("copy:{}", address.email)),
    ])
}

pub(crate) fn attachment_menu(index: usize) -> Menu {
    Menu::new(vec![
        MenuItem::new(&tr!("Open Attachment"), &format!("attachment-open:{index}")),
        MenuItem::new(
            &tr!("Save Attachment…"),
            &format!("attachment-save-as:{index}"),
        ),
        MenuItem::new(
            &tr!("Save to Downloads Folder"),
            &format!("attachment-save:{index}"),
        ),
    ])
}

pub(crate) fn link_alert(
    url: String,
    claimed: &str,
    mut open: impl FnMut(String) + 'static,
) -> Alert {
    let real = link_host(&url).unwrap_or_else(|| url.clone());
    Alert::new(
        &tr!("Open this link?"),
        &tr!(
            "The link says “{claimed}”, but it goes to “{real}”.",
            claimed = claimed.to_owned(),
            real = real
        ),
        move |choice| {
            if choice == "open" {
                open(url.clone());
            }
        },
    )
    .other(&tr!("Open Link"), "open")
    .safe(&tr!("Cancel"), "cancel")
}

pub(crate) fn fetch_remote(
    sources: Vec<String>,
    network: Network,
    deliver: impl FnOnce(Vec<(String, Picture)>) + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name("remote-content".to_owned())
        .spawn(move || {
            let pictures = sources
                .into_iter()
                .filter_map(|source| {
                    let bytes =
                        crate::http::get(&source, crate::http::IMAGES, &network, MAX_PICTURE_BYTES)
                            .inspect_err(|error| tracing::debug!("cannot load {source}: {error}"))
                            .ok()?;
                    crate::picture::decode(&bytes).map(|picture| (source, picture))
                })
                .collect();
            deliver(pictures);
        });
    if let Err(error) = spawned {
        tracing::warn!("cannot load remote content: {error}");
    }
}

pub(crate) fn paint_link_target(
    cx: &mut Context,
    palette: &Palette,
    items: &mut Vec<Item>,
    frame: Rect,
    url: &str,
) {
    let paragraph = cx.text.fitted(
        url,
        TextStyle::role(TextRole::Callout, palette.ink2),
        (frame.width - 2.0 * LINK_TARGET_INSET - 2.0 * LINK_TARGET_PADDING).max(40.0),
    );
    let height = paragraph.height() + LINK_TARGET_PADDING;
    let shown = Rect::new(
        frame.x + LINK_TARGET_INSET,
        frame.y + frame.height - LINK_TARGET_INSET - height,
        paragraph.width() + 2.0 * LINK_TARGET_PADDING,
        height,
    );
    items.push(Item::Fill {
        shape: Shape::rounded(shown, Radius::S),
        paint: palette.raised.into(),
    });
    items.push(Item::Stroke {
        shape: Shape::rounded(shown, Radius::S),
        width: 1.0,
        paint: palette.hairline.into(),
    });
    items.push(Item::Text {
        origin: Point::new(
            shown.x + LINK_TARGET_PADDING,
            shown.y + LINK_TARGET_PADDING / 2.0,
        ),
        paragraph,
    });
}

fn write_new(folder: &Path, name: &str, data: &[u8]) -> io::Result<PathBuf> {
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    for copy in 0..1000 {
        let path = match copy {
            0 => folder.join(name),
            copy => folder.join(format!("{stem} {}{extension}", copy + 1)),
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                let written = file.write_all(data).and_then(|()| file.sync_all());
                if written.is_err() {
                    let _ = std::fs::remove_file(&path);
                }
                return written.map(|()| path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("no free file name"))
}
