mod kill;
mod view;

use std::sync::atomic::Ordering;
use std::time::Instant;

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use accesskit_unix::Adapter;
use bande_graphics::{
    Appearance, Color, Elevation, FRAME_HEADER_HEIGHT, Item, Key, Modifiers, Palette, Pointer,
    PointerButton, Radius, Rect, Scene, Scroll, Shape, TextAlign, title_style,
};
use bande_render::VulkanRenderer;
use bande_widgets::Event;
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::input::keyboard::{Keysym, keysyms};
use smithay::input::pointer::AxisFrame;
use smithay::reexports::calloop::channel::Sender;
use smithay::reexports::wayland_server::{Client, Resource};
use smithay::utils::{Logical, Point, Rectangle, Size};

pub(crate) use self::kill::Process;
use self::view::{CONTENT, Effect, ForceQuitView, Listed, SHELL_KEY};
use crate::L10N;
use crate::shell::toplevel_names;
use crate::state::{ClientState, Compositor, OutputElement, State};

type Frame = Rectangle<f64, Logical>;

const SHADOW: f32 = 32.0;
const ICON_POINTS: f64 = 32.0;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

struct Painted {
    scale: f64,
    buffer: MemoryRenderBuffer,
}

struct Candidate {
    key: String,
    app_id: String,
    title: String,
    clients: Vec<Client>,
}

struct Open {
    view: ForceQuitView,
    screen: usize,
    rect: Frame,
    painted: Option<Painted>,
    dirty: bool,
    pointer_inside: bool,
    held: Option<u32>,
    adapter: Adapter,
}

#[derive(Default)]
pub(crate) struct ForceQuit {
    open: Option<Open>,
    actions: Option<Sender<ActionRequest>>,
}

impl ForceQuit {
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub(crate) fn take_actions(&mut self, actions: Sender<ActionRequest>) {
        self.actions = Some(actions);
    }
}

struct Inactive;

impl ActivationHandler for Inactive {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        None
    }
}

impl DeactivationHandler for Inactive {
    fn deactivate_accessibility(&mut self) {}
}

struct Actions(Option<Sender<ActionRequest>>);

impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        if let Some(actions) = &self.0
            && actions.send(request).is_err()
        {
            tracing::debug!("Force Quit is closed");
        }
    }
}

fn key_of(keysym: u32) -> Option<Key> {
    Some(match keysym {
        keysyms::KEY_Up | keysyms::KEY_KP_Up => Key::Up,
        keysyms::KEY_Down | keysyms::KEY_KP_Down => Key::Down,
        keysyms::KEY_Left | keysyms::KEY_KP_Left => Key::Left,
        keysyms::KEY_Right | keysyms::KEY_KP_Right => Key::Right,
        keysyms::KEY_Home | keysyms::KEY_KP_Home => Key::Home,
        keysyms::KEY_End | keysyms::KEY_KP_End => Key::End,
        keysyms::KEY_Page_Up | keysyms::KEY_KP_Page_Up => Key::PageUp,
        keysyms::KEY_Page_Down | keysyms::KEY_KP_Page_Down => Key::PageDown,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Key::Enter,
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Key::Tab,
        keysyms::KEY_space => Key::Space,
        keysyms::KEY_Escape => Key::Escape,
        keysyms::KEY_BackSpace => Key::Backspace,
        _ => Key::Character(
            Keysym::new(keysym)
                .key_char()
                .filter(|character| !character.is_control())?,
        ),
    })
}

fn pointer_button(button: u32) -> Option<PointerButton> {
    match button {
        BTN_LEFT => Some(PointerButton::Primary),
        BTN_RIGHT => Some(PointerButton::Secondary),
        BTN_MIDDLE => Some(PointerButton::Middle),
        _ => None,
    }
}

fn window_size() -> Size<f64, Logical> {
    Size::from((
        f64::from(CONTENT.0),
        f64::from(CONTENT.1) + f64::from(FRAME_HEADER_HEIGHT),
    ))
}

fn window_scene(content: Vec<Item>, palette: &Palette) -> Scene {
    let (width, height) = (CONTENT.0, CONTENT.1 + FRAME_HEADER_HEIGHT as f32);
    let header = FRAME_HEADER_HEIGHT as f32;
    let shape = Shape::rounded(Rect::new(0.0, 0.0, width, height), Radius::XL);
    let mut items = Item::shadows(shape, Elevation::Frame, palette.scheme, true).to_vec();
    items.push(Item::Clip {
        shape,
        items: vec![
            Item::Fill {
                shape,
                paint: palette.sheet.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, 0.0, width, header)),
                paint: palette.chassis.into(),
            },
            Item::Fill {
                shape: Shape::rect(Rect::new(0.0, header - 1.0, width, 1.0)),
                paint: palette.hairline.into(),
            },
            Item::Translate {
                offset: bande_graphics::Point::new(0.0, header),
                items: content,
            },
        ],
    });
    items.extend(Item::rim(shape, palette));
    Scene {
        width: (width + 2.0 * SHADOW) as u32,
        height: (height + 2.0 * SHADOW) as u32,
        background: Color::TRANSPARENT,
        items: vec![Item::Translate {
            offset: bande_graphics::Point::new(SHADOW, SHADOW),
            items,
        }],
    }
}

fn relaunch_shell(client: &Client) {
    let Some(process) = client
        .get_data::<ClientState>()
        .and_then(|state| state.process.as_ref())
    else {
        return;
    };
    if let Err(error) = process.force_quit(&[]) {
        tracing::warn!("cannot end the shell to start it again: {error}");
    }
}

impl State {
    fn force_quit_appearance(&self) -> Appearance {
        Appearance {
            scheme: self.decorations.scheme,
            reduce_motion: self.reduce_motion,
            text_size: self.window_menus.text_size(),
            ..Appearance::default()
        }
    }

    fn force_quit_candidates(&self) -> Vec<Candidate> {
        let mut found: Vec<Candidate> = Vec::new();
        for window in self.windows.iter().filter(|window| window.size.is_some()) {
            let surface = window.surface();
            let Some(client) = surface.client() else {
                continue;
            };
            if client
                .get_data::<ClientState>()
                .is_some_and(|state| state.role.is_some())
            {
                continue;
            }
            let (title, app_id) = toplevel_names(surface);
            let key = match app_id.is_empty() {
                true => format!("{:?}", client.id()),
                false => app_id.clone(),
            };
            match found.iter_mut().find(|known| known.key == key) {
                Some(known) => {
                    if !known.clients.contains(&client) {
                        known.clients.push(client);
                    }
                    if known.title.is_empty() {
                        known.title = title;
                    }
                }
                None => found.push(Candidate {
                    key,
                    app_id,
                    title,
                    clients: vec![client],
                }),
            }
        }
        for (client, app_id) in self.windowless_apps() {
            match found.iter_mut().find(|known| known.key == app_id) {
                Some(known) => known.clients.push(client),
                None => found.push(Candidate {
                    key: app_id.clone(),
                    app_id,
                    title: String::new(),
                    clients: vec![client],
                }),
            }
        }
        if let Some(shell) = self.desktop_shell.clone().filter(|shell| {
            shell
                .get_data::<ClientState>()
                .is_some_and(|state| !state.departed.load(Ordering::Relaxed))
        }) {
            found.push(Candidate {
                key: SHELL_KEY.to_owned(),
                app_id: String::new(),
                title: String::new(),
                clients: vec![shell],
            });
        }
        found
    }

    fn force_quit_listed(&mut self, screen: usize) -> Vec<Listed> {
        let pixels = (ICON_POINTS * self.screens[screen].scale()).ceil() as u32;
        let mut wanted = Vec::new();
        let mut listed: Vec<Listed> = self
            .force_quit_candidates()
            .into_iter()
            .map(|candidate| {
                let app = self.switcher.app(&candidate.app_id, &candidate.title);
                let name = match (candidate.key == SHELL_KEY, app.name.is_empty()) {
                    (true, _) => L10N.tr("force-quit-shell"),
                    (false, false) => app.name,
                    (false, true) => candidate
                        .clients
                        .iter()
                        .find_map(|client| {
                            client
                                .get_data::<ClientState>()
                                .and_then(|state| state.process.as_ref()?.name())
                        })
                        .unwrap_or_else(|| L10N.tr("force-quit-untitled")),
                };
                let icon = app
                    .icon
                    .as_deref()
                    .and_then(|icon| self.switcher.image(icon, pixels, &mut wanted));
                let responding = !candidate
                    .clients
                    .iter()
                    .any(|client| self.watchdog.is_unresponsive(&client.id()));
                Listed {
                    key: candidate.key,
                    name,
                    icon,
                    responding,
                }
            })
            .collect();
        self.switcher.load_icons(wanted);
        listed.sort_by_cached_key(|app| (app.name.to_lowercase(), app.key.clone()));
        listed
    }

    fn frontmost_key(&self) -> Option<String> {
        let surface = self.key_window()?;
        let client = surface.client()?;
        let (_, app_id) = toplevel_names(&surface);
        Some(match app_id.is_empty() {
            true => format!("{:?}", client.id()),
            false => app_id,
        })
    }

    pub(crate) fn open_force_quit(&mut self) {
        if self.is_locked() {
            return;
        }
        self.watchdog.ping_everyone();
        if self.force_quit.open.is_some() {
            return;
        }
        self.end_switcher();
        self.close_overview();
        let screen = self.pointer_screen();
        let area = self.usable_area(screen).to_f64();
        let size = window_size();
        let rect = Rectangle::new(
            Point::from((
                (area.loc.x + (area.size.w - size.w) / 2.0).round(),
                (area.loc.y + (area.size.h - size.h) / 3.0).max(0.0).round(),
            )),
            size,
        );
        let text = self.labels.get_mut().text().sharing_fonts();
        let mut view = ForceQuitView::new(self.force_quit_appearance(), text);
        view.set_apps(self.force_quit_listed(screen));
        if let Some(front) = self
            .frontmost_key()
            .filter(|key| view.listed().iter().any(|app| app.key == *key))
        {
            view.select(vec![front]);
        }
        view.focus_list();
        let mut adapter =
            Adapter::new(Inactive, Actions(self.force_quit.actions.clone()), Inactive);
        adapter.update_if_active(|| view.accessibility());
        adapter.update_window_focus_state(true);
        self.force_quit.open = Some(Open {
            view,
            screen,
            rect,
            painted: None,
            dirty: true,
            pointer_inside: false,
            held: None,
            adapter,
        });
        self.needs_redraw = true;
    }

    pub(crate) fn close_force_quit(&mut self) {
        if let Some(mut open) = self.force_quit.open.take() {
            open.adapter.update_window_focus_state(false);
            self.needs_redraw = true;
        }
    }

    fn apply_force_quit(&mut self, handled: bool, effects: Vec<Effect>) {
        if let Some(open) = self.force_quit.open.as_mut()
            && handled
        {
            open.dirty = true;
            let view = &open.view;
            open.adapter.update_if_active(|| view.accessibility());
            self.needs_redraw = true;
        }
        for effect in effects {
            match effect {
                Effect::Close => self.close_force_quit(),
                Effect::ForceQuit(keys) => self.force_quit_keys(&keys),
            }
        }
    }

    fn force_quit_keys(&mut self, keys: &[String]) {
        for candidate in self
            .force_quit_candidates()
            .into_iter()
            .filter(|candidate| keys.contains(&candidate.key))
        {
            if candidate.key == SHELL_KEY {
                candidate.clients.iter().for_each(relaunch_shell);
                continue;
            }
            let ids = self.app_ids(&candidate.app_id);
            for client in &candidate.clients {
                self.kill_client(client, &ids);
            }
        }
    }

    pub(crate) fn app_ids(&mut self, app_id: &str) -> Vec<String> {
        let mut ids = vec![app_id.to_owned()];
        if let Some(entry) = self.switcher.app(app_id, "").entry
            && entry != app_id
        {
            ids.push(entry);
        }
        ids
    }

    pub(crate) fn kill_client(&self, client: &Client, app_ids: &[String]) {
        let Some(state) = client.get_data::<ClientState>() else {
            return;
        };
        if state.role.is_some() {
            return;
        }
        let Some(process) = state.process.as_ref() else {
            tracing::warn!("cannot tell which process runs {}", app_ids.join(", "));
            return;
        };
        if process.pid() == std::process::id() as i32 {
            return;
        }
        if let Err(error) = process.force_quit(app_ids) {
            tracing::warn!("cannot force {} to quit: {error}", app_ids.join(", "));
        }
    }

    pub(crate) fn force_quit_key(&mut self, keysym: u32, modifiers: Modifiers) {
        let Some(key) = key_of(keysym) else {
            return;
        };
        let Some(open) = self.force_quit.open.as_mut() else {
            return;
        };
        let (handled, effects) = open.view.handle(&Event::Key { key, modifiers });
        self.apply_force_quit(handled, effects);
    }

    pub(crate) fn force_quit_action(&mut self, request: ActionRequest) {
        let Some(open) = self.force_quit.open.as_mut() else {
            return;
        };
        let (handled, effects) = open.view.perform(&request);
        self.apply_force_quit(handled, effects);
    }

    fn force_quit_point(&self, at: Point<f64, Logical>) -> Option<(bande_graphics::Point, bool)> {
        let open = self.force_quit.open.as_ref()?;
        let local = at - open.rect.loc;
        Some((
            bande_graphics::Point::new(
                local.x as f32,
                (local.y - f64::from(FRAME_HEADER_HEIGHT)) as f32,
            ),
            open.rect.contains(at),
        ))
    }

    pub(crate) fn force_quit_pointer(&mut self, at: Point<f64, Logical>) -> bool {
        let Some((position, inside)) = self.force_quit_point(at) else {
            return false;
        };
        let Some(open) = self.force_quit.open.as_mut() else {
            return false;
        };
        let tracking = inside || open.held.is_some();
        let event = match (tracking, open.pointer_inside) {
            (true, _) => Event::PointerMove(Pointer::at(position)),
            (false, true) => Event::PointerLeave,
            (false, false) => return false,
        };
        open.pointer_inside = inside;
        let (handled, effects) = open.view.handle(&event);
        self.apply_force_quit(handled, effects);
        tracking
    }

    pub(crate) fn force_quit_button(&mut self, button: u32, pressed: bool) -> bool {
        let Some((position, inside)) = self.force_quit_point(self.pointer_location) else {
            return false;
        };
        let Some(open) = self.force_quit.open.as_mut() else {
            return false;
        };
        let kind = pointer_button(button);
        if !pressed {
            if open.held != Some(button) {
                return false;
            }
            open.held = None;
            let pointer = Pointer::at(position).with_button(kind.unwrap_or_default());
            let (handled, effects) = open.view.handle(&Event::PointerUp(pointer));
            self.apply_force_quit(handled, effects);
            return true;
        }
        if !inside {
            if !open.view.is_asking() {
                self.close_force_quit();
            }
            self.swallowed_button = Some(button);
            return true;
        }
        let Some(kind) = kind else {
            self.swallowed_button = Some(button);
            return true;
        };
        open.held = Some(button);
        let pointer = Pointer::at(position).with_button(kind);
        let (handled, effects) = open.view.handle(&Event::PointerDown(pointer));
        self.apply_force_quit(handled, effects);
        true
    }

    pub(crate) fn force_quit_scroll(&mut self, frame: &AxisFrame) -> bool {
        let Some((position, true)) = self.force_quit_point(self.pointer_location) else {
            return false;
        };
        let Some(open) = self.force_quit.open.as_mut() else {
            return false;
        };
        let scroll = Scroll {
            position,
            delta: bande_graphics::Point::new(frame.axis.0 as f32, frame.axis.1 as f32),
            ..Scroll::default()
        };
        let (handled, effects) = open.view.handle(&Event::Scroll(scroll));
        self.apply_force_quit(handled, effects);
        true
    }

    fn force_quit_due(&mut self, now: Instant) -> Option<Instant> {
        let screen = self.force_quit.open.as_ref()?.screen;
        if self.is_locked() || screen >= self.screens.len() {
            self.close_force_quit();
            return None;
        }
        let listed = self.force_quit_listed(screen);
        let appearance = self.force_quit_appearance();
        let open = self.force_quit.open.as_mut()?;
        open.view.set_appearance(appearance);
        if open.view.set_apps(listed) {
            open.dirty = true;
            let view = &open.view;
            open.adapter.update_if_active(|| view.accessibility());
            self.needs_redraw = true;
        }
        let deadline = open.view.deadline();
        if deadline.is_some_and(|deadline| deadline <= now) {
            open.dirty = true;
            self.needs_redraw = true;
        }
        deadline
    }

    pub(crate) fn prepare_force_quit(&mut self, screen: usize) {
        let scale = self.screens[screen].scale();
        let palette = self.force_quit_appearance().palette();
        let State {
            force_quit, labels, ..
        } = self;
        let Some(open) = force_quit
            .open
            .as_mut()
            .filter(|open| open.screen == screen)
        else {
            return;
        };
        let stale = open
            .painted
            .as_ref()
            .is_none_or(|painted| painted.scale != scale);
        if !open.dirty && !stale && !open.view.is_animating() {
            return;
        }
        let content = open.view.paint(CONTENT.0, CONTENT.1);
        let mut scene = window_scene(content, &palette);
        let header = FRAME_HEADER_HEIGHT as f32;
        let text = labels.get_mut().text();
        let title = text.paragraph(
            &L10N.tr("force-quit-title"),
            title_style(&palette, true),
            Some(CONTENT.0),
            TextAlign::Center,
        );
        let top = SHADOW + ((header - title.height()) / 2.0).round();
        scene.items.push(Item::Text {
            origin: bande_graphics::Point::new(SHADOW, top),
            paragraph: title,
        });
        open.painted = labels
            .get_mut()
            .paint(&scene, scale)
            .map(|buffer| Painted { scale, buffer });
        open.dirty = open.view.is_animating();
        if open.dirty {
            self.needs_redraw = true;
        }
    }

    pub(crate) fn force_quit_elements(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Vec<OutputElement> {
        let Some(open) = self
            .force_quit
            .open
            .as_ref()
            .filter(|open| open.screen == screen && !self.is_locked())
        else {
            return Vec::new();
        };
        let Some(painted) = &open.painted else {
            return Vec::new();
        };
        let margin = f64::from(SHADOW);
        let size = Size::<f64, Logical>::from((
            open.rect.size.w + 2.0 * margin,
            open.rect.size.h + 2.0 * margin,
        ));
        let pixels = (
            f64::from(bande_graphics::pixels(size.w as u32, painted.scale)),
            f64::from(bande_graphics::pixels(size.h as u32, painted.scale)),
        );
        let origin = open.rect.loc - Point::from((margin, margin));
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            origin.to_physical(scale).to_i32_round::<i32>().to_f64(),
            &painted.buffer,
            None,
            Some(Rectangle::from_size(pixels.into())),
            Some(size.to_i32_round()),
            Kind::Unspecified,
        )
        .ok()
        .map(OutputElement::Memory)
        .into_iter()
        .collect()
    }
}

impl Compositor {
    pub fn show_force_quit(&mut self) {
        self.state.open_force_quit();
    }

    pub fn force_quit_tick(&mut self, now: Instant) -> Option<Instant> {
        self.state.force_quit_due(now)
    }

    pub fn force_quit_rows(&self) -> Vec<String> {
        self.state
            .force_quit
            .open
            .as_ref()
            .map(|open| open.view.listed().iter().map(Listed::label).collect())
            .unwrap_or_default()
    }

    pub fn force_quit_asks(&self) -> bool {
        self.state
            .force_quit
            .open
            .as_ref()
            .is_some_and(|open| open.view.is_asking())
    }
}
