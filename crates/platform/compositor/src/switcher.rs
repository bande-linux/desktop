use std::collections::HashMap;
use std::time::Duration;

use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::surface::{
    WaylandSurfaceRenderElement, render_elements_from_surface_tree,
};
use smithay::backend::renderer::element::{Id, Kind};
use smithay::input::keyboard::keysyms;
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use unbound_graphics::{
    Color, Image, Item, Paint, Palette, Rect, Scene, Shape, TextAlign, TextStyle,
};
use unbound_render::VulkanRenderer;
use unbound_spring::Spring;
use unbound_xdg::{
    DesktopEntry, ICON_THEMES, Locale, applications, current_desktops, data_dirs, find_icon,
};

use crate::L10N;
use crate::access::Announcer;
use crate::label::Labels;
use crate::motion::{Look, Motion};
use crate::overview::real_rect;
use crate::scene::{FillElement, ShadowElement, fresh, pieces_of, rounded, scaled_pieces};
use crate::shell::toplevel_names;
use crate::state::{Headless, OutputElement, State};

type Frame = Rectangle<f64, Logical>;

const DWELL: Duration = Duration::from_millis(150);
const MARGIN: f64 = 48.0;
const PAD: f64 = 12.0;
const GAP: f64 = 4.0;
const ICON: f64 = 72.0;
const ICON_INSET: f64 = 12.0;
const THUMBNAIL: (f64, f64) = (220.0, 140.0);
const THUMBNAIL_INSET: f64 = 8.0;
const NAME_ROW: f64 = 30.0;
const NAME_SIZE: f32 = 13.0;
const CARD_RADIUS: f64 = 14.0;
const TILE_RADIUS: f64 = 10.0;
const THUMBNAIL_RADIUS: f64 = 6.0;
const ICON_ROUNDING: f32 = 0.225;
const INITIAL_SHARE: f32 = 0.45;
const OPEN_SCALE: f64 = 0.96;
const MINIMIZED_ALPHA: f32 = 0.5;

#[derive(Debug, Clone, PartialEq)]
struct Layout {
    card: Frame,
    tiles: Vec<Frame>,
    contents: Vec<Frame>,
    name: Frame,
}

fn lay_out(tiles: &[Size<f64, Logical>], inset: f64, room: Frame) -> Layout {
    let count = tiles.len() as f64;
    let widths: f64 = tiles.iter().map(|tile| tile.w).sum();
    let tallest = tiles.iter().map(|tile| tile.h).fold(0.0, f64::max);
    let spacing = 2.0 * PAD + (count - 1.0).max(0.0) * GAP;
    let fit = ((room.size.w - 2.0 * MARGIN - spacing) / widths.max(1.0)).clamp(0.0, 1.0);
    let row = tallest * fit;
    let size = Size::from((
        (widths * fit + spacing).round(),
        (PAD + row + NAME_ROW).round(),
    ));
    let loc = Point::from((
        (room.loc.x + (room.size.w - size.w) / 2.0).round(),
        (room.loc.y + (room.size.h - size.h) / 2.0).round(),
    ));
    let card = Rectangle::new(loc, size);
    let mut x = loc.x + PAD;
    let mut placed = Vec::new();
    let mut contents = Vec::new();
    for tile in tiles {
        let tile = tile.upscale(fit);
        let rect = Rectangle::new((x, loc.y + PAD + (row - tile.h) / 2.0).into(), tile);
        let inner = inset * fit;
        contents.push(Rectangle::new(
            rect.loc + Point::from((inner, inner)),
            (tile.w - 2.0 * inner, tile.h - 2.0 * inner).into(),
        ));
        placed.push(rect);
        x += tile.w + GAP;
    }
    Layout {
        card,
        tiles: placed,
        contents,
        name: Rectangle::new(
            (loc.x + PAD, loc.y + PAD + row).into(),
            (size.w - 2.0 * PAD, NAME_ROW).into(),
        ),
    }
}

fn thumbnail_tile(window: Size<f64, Logical>) -> Size<f64, Logical> {
    let fit = (THUMBNAIL.0 / window.w.max(1.0))
        .min(THUMBNAIL.1 / window.h.max(1.0))
        .min(1.0);
    Size::from((
        window.w * fit + 2.0 * THUMBNAIL_INSET,
        window.h * fit + 2.0 * THUMBNAIL_INSET,
    ))
}

fn fitted(rect: Frame, layout: &Frame, shown: &Frame) -> Frame {
    let factor = (shown.size.w / layout.size.w.max(1.0)).min(shown.size.h / layout.size.h.max(1.0));
    let center = |frame: &Frame| frame.loc + frame.size.downscale(2.0).to_point();
    Rectangle::new(
        center(shown) + (rect.loc - center(layout)).upscale(factor),
        rect.size.upscale(factor),
    )
}

fn step(index: usize, count: usize, backwards: bool) -> usize {
    if count == 0 {
        0
    } else if backwards {
        (index + count - 1) % count
    } else {
        (index + 1) % count
    }
}

#[derive(Debug, Clone, PartialEq)]
struct App {
    id: String,
    client: Option<ClientId>,
    name: String,
    icon: Option<String>,
}

impl App {
    fn owns(&self, surface: &WlSurface) -> bool {
        let (_, app_id) = toplevel_names(surface);
        app_id == self.id
            && (self.client.is_none() || surface.client().map(|client| client.id()) == self.client)
    }
}

fn owner_of(surface: &WlSurface) -> (String, String, Option<ClientId>) {
    let (title, app_id) = toplevel_names(surface);
    let client = app_id
        .is_empty()
        .then(|| surface.client().map(|client| client.id()))
        .flatten();
    (title, app_id, client)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Apps,
    Windows { app: usize },
    Cycle,
}

struct Session {
    mode: Mode,
    apps: Vec<App>,
    windows: Vec<WlSurface>,
    selected: usize,
    hidden: Vec<App>,
    screen: usize,
    since: Option<Duration>,
    shown: bool,
}

impl Session {
    fn count(&self) -> usize {
        match self.mode {
            Mode::Apps => self.apps.len(),
            Mode::Windows { .. } | Mode::Cycle => self.windows.len(),
        }
    }
}

#[derive(Clone, PartialEq)]
enum Content {
    Icons(Vec<App>),
    Windows(Vec<WlSurface>),
}

struct Painted {
    buffer: MemoryRenderBuffer,
    scale: f64,
    text: String,
}

struct Card {
    screen: usize,
    layout: Layout,
    content: Content,
    look: Motion,
    ring: Motion,
    closing: bool,
    icons: Option<Painted>,
    name: Option<Painted>,
    ids: [Id; 3],
}

#[derive(Default)]
pub(crate) struct Switcher {
    session: Option<Session>,
    card: Option<Card>,
    labels: Option<Labels>,
    entries: Option<Vec<DesktopEntry>>,
    known: HashMap<String, App>,
    images: HashMap<(String, u32), Option<Image>>,
    announcer: Announcer,
}

impl Switcher {
    pub(crate) fn is_active(&self) -> bool {
        self.session.is_some()
    }

    pub(crate) fn is_shown(&self) -> bool {
        self.session.as_ref().is_some_and(|session| session.shown)
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.session.as_ref().is_some_and(|session| !session.shown)
            || self
                .card
                .as_ref()
                .is_some_and(|card| !card.look.is_settled() || !card.ring.is_settled())
    }

    fn app(&mut self, id: &str, title: &str) -> App {
        if let Some(known) = self.known.get(id) {
            return known.clone();
        }
        let entries = self.entries.get_or_insert_with(|| {
            applications(
                &data_dirs(),
                Locale::from_env().as_ref(),
                &current_desktops(),
            )
        });
        let entry = entries.iter().find(|entry| entry.app_id() == id);
        let name = entry.map(|entry| entry.name.clone()).unwrap_or_else(|| {
            if id.is_empty() {
                title.to_owned()
            } else {
                id.rsplit('.').next().unwrap_or(id).to_owned()
            }
        });
        let app = App {
            id: id.to_owned(),
            client: None,
            name,
            icon: entry.and_then(|entry| entry.icon.clone()),
        };
        if !id.is_empty() {
            self.known.insert(id.to_owned(), app.clone());
        }
        app
    }

    fn image(&mut self, name: &str, pixels: u32) -> Option<Image> {
        self.images
            .entry((name.to_owned(), pixels))
            .or_insert_with(|| {
                let path = find_icon(name, pixels, &ICON_THEMES, &data_dirs())?;
                Image::load(&path, pixels)
            })
            .clone()
    }

    fn paint_icons(&mut self, palette: Palette, scale: f64) {
        let Some(card) = self.card.as_ref() else {
            return;
        };
        let Content::Icons(apps) = card.content.clone() else {
            return;
        };
        let text = apps
            .iter()
            .map(|app| app.id.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        if card
            .icons
            .as_ref()
            .is_some_and(|painted| painted.scale == scale && painted.text == text)
        {
            return;
        }
        let layout = card.layout.clone();
        let origin = layout.card.loc;
        let mut items = Vec::new();
        for (app, content) in apps.iter().zip(&layout.contents) {
            let rect = Rect::new(
                (content.loc.x - origin.x) as f32,
                (content.loc.y - origin.y) as f32,
                content.size.w as f32,
                content.size.h as f32,
            );
            let pixels = (content.size.w * scale).ceil() as u32;
            match app
                .icon
                .as_deref()
                .and_then(|icon| self.image(icon, pixels))
            {
                Some(image) => items.push(Item::Image { rect, image }),
                None => items.extend(self.placeholder(app, rect, palette)),
            }
        }
        let scene = Scene {
            width: layout.card.size.w as u32,
            height: layout.card.size.h as u32,
            background: Color::TRANSPARENT,
            items,
        };
        let buffer = self
            .labels
            .get_or_insert_with(Labels::default)
            .paint(&scene, scale);
        if let Some(card) = self.card.as_mut() {
            card.icons = buffer.map(|buffer| Painted {
                buffer,
                scale,
                text,
            });
        }
    }

    fn placeholder(&mut self, app: &App, rect: Rect, palette: Palette) -> Vec<Item> {
        let initial = app
            .name
            .chars()
            .next()
            .map(|character| character.to_uppercase().collect::<String>())
            .unwrap_or_default();
        let paragraph = self
            .labels
            .get_or_insert_with(Labels::default)
            .text()
            .paragraph(
                &initial,
                TextStyle::new(rect.height * INITIAL_SHARE, palette.surface_elevated)
                    .weight(TextStyle::SEMIBOLD),
                Some(rect.width),
                TextAlign::Center,
            );
        let top = rect.y + (rect.height - paragraph.height()) / 2.0;
        vec![
            Item::Fill {
                shape: Shape::rounded(rect, rect.width * ICON_ROUNDING),
                paint: Paint::Solid(palette.label_muted),
            },
            Item::Text {
                origin: unbound_graphics::Point::new(rect.x, top),
                paragraph,
            },
        ]
    }

    fn paint_name(&mut self, text: String, palette: Palette, scale: f64) {
        let Some(card) = self.card.as_ref() else {
            return;
        };
        if card
            .name
            .as_ref()
            .is_some_and(|painted| painted.scale == scale && painted.text == text)
        {
            return;
        }
        let row = card.layout.name;
        let labels = self.labels.get_or_insert_with(Labels::default);
        let paragraph = labels.text().paragraph(
            &text,
            TextStyle::new(NAME_SIZE, palette.label_primary).weight(TextStyle::SEMIBOLD),
            Some(row.size.w as f32),
            TextAlign::Center,
        );
        let top = ((NAME_ROW as f32 - paragraph.height()) / 2.0).max(0.0);
        let scene = Scene {
            width: row.size.w as u32,
            height: row.size.h as u32,
            background: Color::TRANSPARENT,
            items: vec![Item::Clip {
                shape: Shape::rect(Rect::new(0.0, 0.0, row.size.w as f32, row.size.h as f32)),
                items: vec![Item::Text {
                    origin: unbound_graphics::Point::new(0.0, top),
                    paragraph,
                }],
            }],
        };
        let buffer = labels.paint(&scene, scale);
        if let Some(card) = self.card.as_mut() {
            card.name = buffer.map(|buffer| Painted {
                buffer,
                scale,
                text,
            });
        }
    }
}

impl State {
    fn running_apps(&mut self) -> Vec<App> {
        let key = self.key_window().map(|surface| owner_of(&surface));
        let mut found: Vec<(String, String, Option<ClientId>)> = Vec::new();
        let owners = self
            .windows
            .iter()
            .rev()
            .filter(|window| window.size.is_some())
            .map(|window| owner_of(window.surface()));
        for owner in owners {
            if !found
                .iter()
                .any(|known| known.1 == owner.1 && known.2 == owner.2)
            {
                found.push(owner);
            }
        }
        if let Some(index) = key.and_then(|key| {
            found
                .iter()
                .position(|known| known.1 == key.1 && known.2 == key.2)
        }) {
            let current = found.remove(index);
            found.insert(0, current);
        }
        found
            .into_iter()
            .map(|(title, app_id, client)| App {
                client,
                ..self.switcher.app(&app_id, &title)
            })
            .collect()
    }

    fn windows_of(&self, app: &App, present_only: bool) -> Vec<WlSurface> {
        self.windows
            .iter()
            .rev()
            .filter(|window| window.size.is_some())
            .filter(|window| !present_only || self.present(window))
            .filter(|window| app.owns(window.surface()))
            .map(|window| window.surface().clone())
            .collect()
    }

    pub(crate) fn bring_forward(&mut self, surface: &WlSurface) {
        let (title, app_id, client) = owner_of(surface);
        let app = App {
            client,
            ..self.switcher.app(&app_id, &title)
        };
        self.show_windows(self.windows_of(&app, false));
        self.unminimize(surface);
        if self.exclusive_layer.is_none() {
            self.focus(Some(surface.clone()));
        }
        self.needs_redraw = true;
    }

    pub(crate) fn switch_apps(&mut self, backwards: bool) {
        if let Some(session) = &self.switcher.session
            && session.mode != Mode::Cycle
        {
            self.step_switcher(backwards);
            return;
        }
        self.end_switcher();
        let apps = self.running_apps();
        if apps.is_empty() {
            return;
        }
        let selected = if backwards {
            apps.len() - 1
        } else {
            usize::from(self.key_window().is_some() && apps.len() > 1)
        };
        self.close_overview();
        self.switcher.session = Some(Session {
            mode: Mode::Apps,
            apps,
            windows: Vec::new(),
            selected,
            hidden: Vec::new(),
            screen: self.pointer_screen(),
            since: None,
            shown: false,
        });
        self.needs_redraw = true;
    }

    pub(crate) fn cycle_windows(&mut self, backwards: bool) {
        match self.switcher.session.as_ref().map(|session| session.mode) {
            Some(Mode::Apps) => {
                self.show_switcher_windows();
                return;
            }
            Some(Mode::Windows { .. } | Mode::Cycle) => {
                self.step_switcher(backwards);
                return;
            }
            None => {}
        }
        let Some(key) = self.key_window() else {
            return;
        };
        let (title, app_id, client) = owner_of(&key);
        let app = App {
            client,
            ..self.switcher.app(&app_id, &title)
        };
        let windows = self.windows_of(&app, true);
        if windows.len() < 2 {
            return;
        }
        self.close_overview();
        self.switcher.session = Some(Session {
            mode: Mode::Cycle,
            apps: Vec::new(),
            windows,
            selected: 0,
            hidden: Vec::new(),
            screen: self.pointer_screen(),
            since: None,
            shown: false,
        });
        self.step_switcher(backwards);
    }

    fn step_switcher(&mut self, backwards: bool) {
        let Some(session) = self.switcher.session.as_mut() else {
            return;
        };
        session.selected = step(session.selected, session.count(), backwards);
        self.switcher_selection_changed();
    }

    fn select_in_switcher(&mut self, index: usize) {
        let Some(session) = self.switcher.session.as_mut() else {
            return;
        };
        if session.selected != index && index < session.count() {
            session.selected = index;
            self.switcher_selection_changed();
        }
    }

    fn switcher_selection_changed(&mut self) {
        let Some(session) = self.switcher.session.as_ref() else {
            return;
        };
        if session.mode == Mode::Cycle
            && let Some(window) = session.windows.get(session.selected).cloned()
        {
            self.bring_forward(&window);
        }
        self.refresh_card();
    }

    fn show_switcher_windows(&mut self) {
        let Some(session) = self.switcher.session.as_ref() else {
            return;
        };
        let Mode::Apps = session.mode else {
            return;
        };
        let app = session.selected;
        let Some(chosen) = session.apps.get(app).cloned() else {
            return;
        };
        let windows = self.windows_of(&chosen, false);
        if windows.is_empty() {
            return;
        }
        if let Some(session) = self.switcher.session.as_mut() {
            session.mode = Mode::Windows { app };
            session.windows = windows;
            session.selected = 0;
            session.shown = true;
        }
        self.refresh_card();
    }

    fn show_switcher_apps(&mut self) {
        let Some(session) = self.switcher.session.as_mut() else {
            return;
        };
        if let Mode::Windows { app } = session.mode {
            session.mode = Mode::Apps;
            session.selected = app;
            session.windows.clear();
            self.refresh_card();
        }
    }

    pub(crate) fn switcher_key(&mut self, keysym: u32, shift: bool) {
        match keysym {
            keysyms::KEY_Escape => self.end_switcher(),
            keysyms::KEY_Return | keysyms::KEY_KP_Enter => self.commit_switcher(),
            keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => self.step_switcher(shift),
            keysyms::KEY_Right => self.step_switcher(false),
            keysyms::KEY_Left => self.step_switcher(true),
            keysyms::KEY_Down | keysyms::KEY_grave | keysyms::KEY_asciitilde => {
                if self.switcher.session.as_ref().map(|session| session.mode) == Some(Mode::Apps) {
                    self.show_switcher_windows();
                } else {
                    self.step_switcher(shift);
                }
            }
            keysyms::KEY_Up => self.show_switcher_apps(),
            keysyms::KEY_q | keysyms::KEY_Q => self.quit_selected_app(),
            keysyms::KEY_h | keysyms::KEY_H => {
                if let Some(app) = self.selected_app() {
                    self.hide_windows(self.windows_of(&app, false));
                    if let Some(session) = self.switcher.session.as_mut() {
                        session.hidden.push(app);
                    }
                }
            }
            _ => {}
        }
    }

    fn selected_app(&self) -> Option<App> {
        let session = self.switcher.session.as_ref()?;
        match session.mode {
            Mode::Apps => session.apps.get(session.selected).cloned(),
            Mode::Windows { app } => session.apps.get(app).cloned(),
            Mode::Cycle => None,
        }
    }

    fn quit_selected_app(&mut self) {
        let Some(app) = self.selected_app() else {
            return;
        };
        if app.client.is_some() {
            let owner = self
                .windows_of(&app, false)
                .first()
                .and_then(|surface| surface.client());
            self.quit_app(owner);
        } else {
            self.quit_app_named(&app.id);
        }
        let Some(session) = self.switcher.session.as_mut() else {
            return;
        };
        if let Mode::Windows { app } = session.mode {
            session.mode = Mode::Apps;
            session.selected = app;
            session.windows.clear();
        }
        session.apps.retain(|known| *known != app);
        if session.apps.is_empty() {
            self.end_switcher();
            return;
        }
        session.selected = session.selected.min(session.apps.len() - 1);
        self.refresh_card();
    }

    pub(crate) fn commit_switcher(&mut self) {
        let Some(session) = self.switcher.session.as_ref() else {
            return;
        };
        let chosen_app = (session.mode == Mode::Apps)
            .then(|| session.apps.get(session.selected).cloned())
            .flatten()
            .filter(|app| !session.hidden.contains(app));
        let chosen_window = matches!(session.mode, Mode::Windows { .. })
            .then(|| session.windows.get(session.selected).cloned())
            .flatten();
        self.end_switcher();
        if let Some(app) = chosen_app {
            match app.client {
                None => self.activate_app(&app.id),
                Some(_) => {
                    if let Some(front) = self.windows_of(&app, false).first() {
                        self.bring_forward(front);
                    }
                }
            }
        }
        if let Some(window) = chosen_window {
            self.bring_forward(&window);
        }
    }

    pub(crate) fn end_switcher(&mut self) {
        if self.switcher.session.take().is_none() {
            return;
        }
        self.switcher.announcer.withdraw();
        if let Some(card) = self.switcher.card.as_mut() {
            card.closing = true;
            let hidden = if self.reduce_motion {
                Look::hidden(card.layout.card)
            } else {
                Look::hidden(card.layout.card).scaled(OPEN_SCALE)
            };
            card.look.retarget(hidden);
        }
        self.needs_redraw = true;
    }

    fn switcher_content(&self, session: &Session) -> Content {
        match session.mode {
            Mode::Apps => Content::Icons(session.apps.clone()),
            Mode::Windows { .. } | Mode::Cycle => Content::Windows(session.windows.clone()),
        }
    }

    fn switcher_layout(&self, session: &Session, content: &Content) -> Layout {
        let area = self.usable_area(session.screen);
        let room = Rectangle::new(area.location.to_f64(), area.size.to_f64());
        match content {
            Content::Icons(apps) => lay_out(
                &vec![Size::from((ICON + 2.0 * ICON_INSET, ICON + 2.0 * ICON_INSET)); apps.len()],
                ICON_INSET,
                room,
            ),
            Content::Windows(windows) => {
                let tiles: Vec<Size<f64, Logical>> = windows
                    .iter()
                    .map(|surface| {
                        self.window_index(surface)
                            .and_then(|index| real_rect(&self.windows[index]))
                            .map_or(Size::from((THUMBNAIL.0, THUMBNAIL.1)), |rect| rect.size)
                    })
                    .map(thumbnail_tile)
                    .collect();
                lay_out(&tiles, THUMBNAIL_INSET, room)
            }
        }
    }

    fn announce_switcher(&mut self) {
        let Some(session) = self
            .switcher
            .session
            .as_ref()
            .filter(|session| session.shown)
        else {
            return;
        };
        let selected = session.selected;
        let (title, items) = match session.mode {
            Mode::Apps => (
                L10N.tr("app-switcher"),
                session.apps.iter().map(|app| app.name.clone()).collect(),
            ),
            Mode::Windows { .. } | Mode::Cycle => {
                let named: Vec<(String, String)> =
                    session.windows.iter().map(toplevel_names).collect();
                let items = named
                    .into_iter()
                    .map(|(title, app_id)| {
                        if title.is_empty() {
                            self.switcher.app(&app_id, &title).name
                        } else {
                            title
                        }
                    })
                    .collect::<Vec<_>>();
                (L10N.tr("window-switcher"), items)
            }
        };
        self.switcher.announcer.announce(&title, &items, selected);
    }

    fn refresh_card(&mut self) {
        self.refresh_card_look();
        self.announce_switcher();
    }

    fn refresh_card_look(&mut self) {
        self.needs_redraw = true;
        let Some(session) = self.switcher.session.as_ref() else {
            return;
        };
        if !session.shown {
            return;
        }
        let content = self.switcher_content(session);
        let layout = self.switcher_layout(session, &content);
        let selected = layout.tiles.get(session.selected).copied();
        let screen = session.screen;
        let reduce = self.reduce_motion;
        let spring = if reduce { Spring::FADE } else { Spring::QUICK };
        match self.switcher.card.as_mut().filter(|card| !card.closing) {
            Some(card) => {
                let rebuilt = card.content != content || card.layout != layout;
                card.look.retarget(Look::shown(layout.card));
                if let Some(tile) = selected {
                    if rebuilt || reduce {
                        card.ring = Motion::new(spring, Look::shown(tile), Look::shown(tile));
                    } else {
                        card.ring.retarget(Look::shown(tile));
                    }
                }
                if rebuilt {
                    card.icons = None;
                }
                card.layout = layout;
                card.content = content;
            }
            None => {
                let from = if reduce {
                    Look::hidden(layout.card)
                } else {
                    Look::hidden(layout.card).scaled(OPEN_SCALE)
                };
                let tile = selected.unwrap_or(layout.card);
                self.switcher.card = Some(Card {
                    screen,
                    look: Motion::new(spring, from, Look::shown(layout.card)),
                    ring: Motion::new(spring, Look::shown(tile), Look::shown(tile)),
                    layout,
                    content,
                    closing: false,
                    icons: None,
                    name: None,
                    ids: [Id::new(), Id::new(), Id::new()],
                });
            }
        }
    }

    fn selected_name(&mut self) -> String {
        let Some(session) = self.switcher.session.as_ref() else {
            return self
                .switcher
                .card
                .as_ref()
                .and_then(|card| card.name.as_ref())
                .map(|painted| painted.text.clone())
                .unwrap_or_default();
        };
        match session.mode {
            Mode::Apps => session
                .apps
                .get(session.selected)
                .map(|app| app.name.clone())
                .unwrap_or_default(),
            Mode::Windows { .. } | Mode::Cycle => {
                let Some((title, app_id)) =
                    session.windows.get(session.selected).map(toplevel_names)
                else {
                    return String::new();
                };
                if title.is_empty() {
                    self.switcher.app(&app_id, &title).name
                } else {
                    title
                }
            }
        }
    }

    pub(crate) fn animate_switcher(&mut self, time: Duration) {
        if let Some(session) = self.switcher.session.as_mut()
            && !session.shown
        {
            let since = *session.since.get_or_insert(time);
            if time.saturating_sub(since) >= DWELL {
                session.shown = true;
                self.refresh_card();
            }
            self.needs_redraw = true;
        }
        let refresh = self.main_screen().refresh();
        if let Some(card) = self.switcher.card.as_mut() {
            card.look.advance(time, refresh);
            card.ring.advance(time, refresh);
            if card.closing && card.look.is_settled() {
                self.switcher.card = None;
            }
            self.needs_redraw = true;
        }
    }

    fn switcher_hit(&self, at: Point<f64, Logical>) -> Option<Option<usize>> {
        let card = self.switcher.card.as_ref().filter(|card| !card.closing)?;
        let shown = card.look.look().rect;
        if !shown.contains(at) {
            return Some(None);
        }
        Some(
            card.layout
                .tiles
                .iter()
                .position(|tile| fitted(*tile, &card.layout.card, &shown).contains(at)),
        )
    }

    pub(crate) fn switcher_pointer(&mut self, at: Point<f64, Logical>) {
        if let Some(Some(index)) = self.switcher_hit(at)
            && self.switcher.session.as_ref().map(|session| session.mode) != Some(Mode::Cycle)
        {
            self.select_in_switcher(index);
        }
    }

    pub(crate) fn switcher_click(&mut self) -> bool {
        if !self.switcher.is_shown() {
            return false;
        }
        match self.switcher_hit(self.pointer_location) {
            Some(Some(index)) => {
                let cycling =
                    self.switcher.session.as_ref().map(|session| session.mode) == Some(Mode::Cycle);
                if let Some(session) = self.switcher.session.as_mut() {
                    session.selected = index;
                }
                if cycling {
                    self.switcher_selection_changed();
                    self.end_switcher();
                } else {
                    self.commit_switcher();
                }
            }
            Some(None) | None => self.end_switcher(),
        }
        true
    }

    fn thumbnail_elements(
        &self,
        renderer: &mut VulkanRenderer,
        surface: &WlSurface,
        to: Frame,
        alpha: f32,
        scale: f64,
    ) -> Vec<OutputElement> {
        let Some(window) = self.window_index(surface).map(|index| &self.windows[index]) else {
            return Vec::new();
        };
        let Some(rect) = real_rect(window) else {
            return Vec::new();
        };
        let surfaces: Vec<WaylandSurfaceRenderElement<VulkanRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                window.surface(),
                window
                    .surface_origin()
                    .to_f64()
                    .to_physical(scale)
                    .to_i32_round::<i32>(),
                scale,
                1.0,
                Kind::Unspecified,
            );
        let mut pieces = pieces_of(&surfaces, scale);
        pieces.extend(window.title_bar_piece(scale));
        let small = to.to_physical(scale);
        let alpha = if window.is_minimized() || window.hidden {
            alpha * MINIMIZED_ALPHA
        } else {
            alpha
        };
        scaled_pieces(
            &fresh(pieces),
            rect.to_physical(scale),
            small,
            rounded(small, THUMBNAIL_RADIUS * scale),
            alpha,
        )
        .collect()
    }

    pub(crate) fn prepare_switcher(&mut self, screen: usize) {
        if !self
            .switcher
            .card
            .as_ref()
            .is_some_and(|card| card.screen == screen)
        {
            return;
        }
        let scale = self.screens[screen].scale();
        let palette = Palette::for_scheme(self.decorations.scheme);
        let name = self.selected_name();
        self.switcher.paint_icons(palette, scale);
        self.switcher.paint_name(name, palette, scale);
    }

    pub(crate) fn switcher_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        let Some(card) = self
            .switcher
            .card
            .as_ref()
            .filter(|card| card.screen == screen && !self.is_locked())
        else {
            return Vec::new();
        };
        let look = card.look.look();
        let alpha = look.alpha as f32;
        if alpha <= 0.0 {
            return Vec::new();
        }
        let palette = Palette::for_scheme(self.decorations.scheme);
        let fill = |color: Color| {
            let [r, g, b] =
                [color.r, color.g, color.b].map(|channel| f32::from(channel) / 255.0 * alpha);
            [r, g, b, alpha]
        };
        let layout = &card.layout;
        let place = |rect: Frame| fitted(rect, &layout.card, &look.rect);
        let factor = look.rect.size.w / layout.card.size.w.max(1.0);
        let mut elements = Vec::new();
        let mut painted = |painted: &Option<Painted>, rect: Frame| {
            let painted = painted.as_ref().filter(|painted| painted.scale == scale)?;
            let shown = place(rect);
            MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                shown.loc.to_physical(scale).to_i32_round::<i32>().to_f64(),
                &painted.buffer,
                Some(alpha),
                Some(Rectangle::from_size(Size::from((
                    f64::from(unbound_graphics::pixels(rect.size.w as u32, scale)),
                    f64::from(unbound_graphics::pixels(rect.size.h as u32, scale)),
                )))),
                Some(shown.size.to_i32_round()),
                Kind::Unspecified,
            )
            .ok()
            .map(OutputElement::Memory)
        };
        elements.extend(painted(&card.name, layout.name));
        match &card.content {
            Content::Icons(_) => elements.extend(painted(&card.icons, layout.card)),
            Content::Windows(windows) => {
                for (surface, content) in windows.iter().zip(&layout.contents) {
                    elements.extend(self.thumbnail_elements(
                        renderer,
                        surface,
                        place(*content),
                        alpha,
                        scale,
                    ));
                }
            }
        }
        let ring = place(card.ring.look().rect);
        elements.push(OutputElement::Fill(FillElement::new(
            card.ids[0].clone(),
            rounded(ring.to_physical(scale), TILE_RADIUS * factor * scale),
            fill(palette.separator_color),
        )));
        let shape = rounded(look.rect.to_physical(scale), CARD_RADIUS * factor * scale);
        elements.push(OutputElement::Fill(FillElement::new(
            card.ids[1].clone(),
            shape,
            fill(palette.surface_elevated),
        )));
        elements.push(OutputElement::Shadow(ShadowElement::new(
            card.ids[2].clone(),
            shape,
            true,
            scale,
            alpha,
        )));
        elements
    }
}

impl Headless {
    pub fn switcher_shown(&self) -> bool {
        self.state.switcher.is_shown()
    }

    pub fn switcher_tiles(&self) -> Vec<(f64, f64)> {
        self.state
            .switcher
            .card
            .iter()
            .flat_map(|card| {
                let shown = card.look.look().rect;
                card.layout.tiles.iter().map(move |tile| {
                    let tile = fitted(*tile, &card.layout.card, &shown);
                    (
                        tile.loc.x + tile.size.w / 2.0,
                        tile.loc.y + tile.size.h / 2.0,
                    )
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room() -> Frame {
        Rectangle::new((0.0, 30.0).into(), (1280.0, 770.0).into())
    }

    #[test]
    fn the_card_is_centred_and_its_tiles_sit_side_by_side_inside_it() {
        let tile = Size::from((96.0, 96.0));
        let layout = lay_out(&[tile; 3], ICON_INSET, room());

        let card = layout.card;
        assert_eq!(card.size.w, 3.0 * 96.0 + 2.0 * GAP + 2.0 * PAD);
        assert!((card.loc.x + card.size.w / 2.0 - 640.0).abs() <= 0.5);
        assert!((card.loc.y + card.size.h / 2.0 - 415.0).abs() <= 0.5);
        for pair in layout.tiles.windows(2) {
            assert_eq!(pair[1].loc.x - (pair[0].loc.x + pair[0].size.w), GAP);
        }
        assert_eq!(layout.contents[0].size, Size::from((ICON, ICON)));
    }

    #[test]
    fn many_apps_shrink_to_fit_the_display() {
        let tile = Size::from((96.0, 96.0));
        let layout = lay_out(&[tile; 40], ICON_INSET, room());

        assert!(layout.card.loc.x >= MARGIN - 0.5);
        assert!(layout.card.loc.x + layout.card.size.w <= 1280.0 - MARGIN + 0.5);
        let last = layout.tiles[39];
        assert!(last.loc.x + last.size.w <= layout.card.loc.x + layout.card.size.w);
        assert!(layout.tiles[0].size.w < 96.0);
    }

    #[test]
    fn a_thumbnail_keeps_the_window_shape_within_its_box() {
        let wide = thumbnail_tile(Size::from((1600.0, 400.0)));
        let small = thumbnail_tile(Size::from((100.0, 50.0)));

        assert!((wide.w - (THUMBNAIL.0 + 2.0 * THUMBNAIL_INSET)).abs() < 1e-9);
        assert!(((wide.h - 2.0 * THUMBNAIL_INSET) - THUMBNAIL.0 / 4.0).abs() < 1e-9);
        assert_eq!(small, Size::from((116.0, 66.0)));
    }

    #[test]
    fn stepping_wraps_around_in_both_directions() {
        assert_eq!(step(2, 3, false), 0);
        assert_eq!(step(0, 3, true), 2);
    }
}
