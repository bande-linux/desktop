use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::{Request, Screen, Sender};
use unbound_inbox::{Inbox, Outbox};
use unbound_notify::Presentation;
use unbound_notify::client::NotificationCenter;
use unbound_toolkit::{
    App, Edges, Image, KeyboardFocus, LayerKind, LayerOptions, Panel, Rect, Role, Scene,
    TextSystem, Window, WindowEvent,
};
use unbound_xdg::{
    DesktopEntry, Locale, applications, current_desktops, data_dirs, find_icon, launch,
};
use zbus::blocking::Connection;

const ICON_PIXELS: u32 = 64;
const AGE_REFRESH: Duration = Duration::from_secs(30);

enum Message {
    Presentation(Presentation),
    Toggle,
    Focus { app: String, token: String },
}

enum Outgoing {
    Activate {
        id: u32,
        action: String,
        token: String,
        app: String,
    },
    Call(Request),
}

pub struct Notifications {
    screen: Screen,
    inbox: Inbox<Message>,
    outgoing: mpsc::Sender<Outgoing>,
    entries: Vec<DesktopEntry>,
    region: Option<Option<Vec<Rect>>>,
    keyboard: Option<bool>,
    hide_pending: bool,
    next_refresh: Option<Instant>,
}

fn report<E: std::fmt::Display>(what: &str, result: Result<(), E>) {
    if let Err(error) = result {
        tracing::warn!("cannot {what}: {error}");
    }
}

fn sender_lookup(
    entries: Vec<DesktopEntry>,
) -> impl FnMut(&unbound_notify::Notification, &str) -> Sender {
    let mut icons: HashMap<String, Option<Image>> = HashMap::new();
    let directories = data_dirs();
    move |notification, group| {
        let entry = entries.iter().find(|entry| {
            entry.app_id() == group || entry.name.eq_ignore_ascii_case(&notification.app_name)
        });
        let name = entry.map_or_else(
            || {
                if notification.app_name.is_empty() {
                    group.to_owned()
                } else {
                    notification.app_name.clone()
                }
            },
            |entry| entry.name.clone(),
        );
        let icon = [
            Some(notification.app_icon.as_str()).filter(|icon| !icon.is_empty()),
            entry.and_then(|entry| entry.icon.as_deref()),
        ]
        .into_iter()
        .flatten()
        .find_map(|icon_name| {
            icons
                .entry(icon_name.to_owned())
                .or_insert_with(|| {
                    let path = if icon_name.starts_with('/') {
                        Some(Path::new(icon_name).to_path_buf())
                    } else {
                        find_icon(icon_name, ICON_PIXELS, &[], &directories)
                    };
                    path.and_then(|path| Image::load(&path, ICON_PIXELS))
                })
                .clone()
        });
        Sender { name, icon }
    }
}

impl Notifications {
    fn sync(&mut self, window: &mut Window) {
        let now = Instant::now();
        let visible = self.screen.visible(now) || self.screen.animating(now);
        if visible && !window.is_visible() {
            report("show the notifications", window.show());
            self.region = None;
            self.keyboard = None;
        }
        self.hide_pending = !visible && window.is_visible();
        let region = self.screen.input_region();
        if self.region.as_ref() != Some(&region) {
            window.set_input_region(region.as_deref());
            self.region = Some(region);
        }
        let keyboard = self.screen.wants_keyboard();
        if self.keyboard != Some(keyboard) {
            let focus = if keyboard {
                KeyboardFocus::Exclusive
            } else {
                KeyboardFocus::None
            };
            report("change keyboard focus", window.set_keyboard_focus(focus));
            self.keyboard = Some(keyboard);
        }
        self.next_refresh = self.screen.is_open().then(|| now + AGE_REFRESH);
        window.request_redraw();
    }

    fn carry_out(&mut self, window: &mut Window) {
        for request in self.screen.take_requests() {
            let outgoing = match request {
                Request::Activate { id, action, group } => {
                    let token = window.activation_token(&group).unwrap_or_default();
                    Outgoing::Activate {
                        id,
                        action,
                        token,
                        app: group,
                    }
                }
                request => Outgoing::Call(request),
            };
            let _ = self.outgoing.send(outgoing);
        }
        self.sync(window);
    }

    fn focus(&self, window: &mut Window, app: &str, token: &str) {
        if window.running_apps().iter().any(|running| running == app) {
            report("bring the app forward", window.activate_app(app));
            return;
        }
        if let Some(entry) = self.entries.iter().find(|entry| entry.app_id() == app) {
            let token = Some(token).filter(|token| !token.is_empty());
            if let Err(error) = launch(entry, token) {
                tracing::warn!("cannot open {}: {error}", entry.name);
            }
        }
    }
}

impl App for Notifications {
    fn scene(&mut self, window: &mut Window) -> Scene {
        let now = Instant::now();
        let (width, height) = window.size();
        self.screen.set_size(width, height);
        self.screen.settle(now);
        if !self.screen.visible(now) && !self.screen.animating(now) {
            self.hide_pending = true;
        }
        self.screen.scene(now)
    }

    fn event(&mut self, window: &mut Window, event: WindowEvent) {
        let now = Instant::now();
        match event {
            WindowEvent::Input(input) => {
                if self.screen.handle(&input, now) {
                    window.request_redraw();
                }
                self.carry_out(window);
            }
            WindowEvent::AppearanceChanged(appearance) => {
                self.screen
                    .set_appearance(appearance.palette(), appearance.reduce_motion, now);
                window.request_redraw();
            }
            WindowEvent::PanelAnchor {
                panel: Panel::NotificationCenter,
                edge,
                rect,
                ..
            } => {
                self.screen.set_anchor(edge, rect);
                window.request_redraw();
            }
            _ => {}
        }
    }

    fn sources(&self) -> Vec<BorrowedFd<'_>> {
        vec![self.inbox.fd()]
    }

    fn source_ready(&mut self, window: &mut Window, _index: usize) {
        let now = Instant::now();
        for message in self.inbox.drain() {
            match message {
                Message::Presentation(presentation) => self.screen.present(presentation, now),
                Message::Toggle => self.screen.toggle(now),
                Message::Focus { app, token } => self.focus(window, &app, &token),
            }
        }
        self.carry_out(window);
    }

    fn animating(&self) -> bool {
        self.screen.animating(Instant::now())
    }

    fn deadline(&self) -> Option<Instant> {
        if self.hide_pending {
            return Some(Instant::now());
        }
        self.next_refresh
    }

    fn tick(&mut self, window: &mut Window, now: Instant) {
        if self.hide_pending && !self.screen.visible(now) && !self.screen.animating(now) {
            self.hide_pending = false;
            report("hide the notifications", window.hide());
            return;
        }
        self.hide_pending = false;
        if self.next_refresh.is_some_and(|refresh| refresh <= now) {
            self.screen.rebuild(now);
            self.sync(window);
        }
    }
}

fn follow(connection: &Connection, outbox: &Outbox<Message>) -> zbus::Result<()> {
    let center = NotificationCenter::new(connection)?;
    let changes = center.receive_presentation_changed()?;
    outbox.send(Message::Presentation(Presentation::from_wire(
        center.get_presentation()?,
    )));
    for change in changes {
        if let Ok(arguments) = change.args() {
            outbox.send(Message::Presentation(Presentation::from_wire(
                arguments.presentation,
            )));
        }
    }
    Ok(())
}

fn serve(connection: &Connection, outgoing: &Receiver<Outgoing>, outbox: &Outbox<Message>) {
    let center = match NotificationCenter::new(connection) {
        Ok(center) => center,
        Err(error) => {
            tracing::warn!("cannot reach the notification daemon: {error}");
            return;
        }
    };
    for request in outgoing {
        let result = match request {
            Outgoing::Activate {
                id,
                action,
                token,
                app,
            } => center.activate(id, &action, &token).map(|invoked| {
                if !invoked {
                    outbox.send(Message::Focus { app, token });
                }
            }),
            Outgoing::Call(Request::Dismiss(id)) => center.dismiss(id),
            Outgoing::Call(Request::HideBanner(id)) => center.hide_banner(id),
            Outgoing::Call(Request::Clear(group)) => center.clear(&group),
            Outgoing::Call(Request::ClearAll) => center.clear_all(),
            Outgoing::Call(Request::HoldBanners(held)) => center.hold_banners(held),
            Outgoing::Call(Request::Activate { .. }) => Ok(()),
        };
        report("reach the notification daemon", result);
    }
}

pub fn role() -> Role {
    Role::Layer(LayerOptions {
        layer: LayerKind::Top,
        anchor: Edges::ALL,
        exclusive_zone: 0,
        keyboard: KeyboardFocus::None,
        namespace: "notifications".to_owned(),
    })
}

impl Notifications {
    pub fn new(
        window: &mut Window,
        text: TextSystem,
        session: Option<&Connection>,
    ) -> std::io::Result<Self> {
        let entries = applications(
            &data_dirs(),
            Locale::from_env().as_ref(),
            &current_desktops(),
        );
        let inbox = Inbox::new()?;
        let (outgoing, requests) = mpsc::channel();
        if let Some(connection) = session {
            let following = (connection.clone(), inbox.outbox());
            thread::spawn(move || {
                while let Err(error) = follow(&following.0, &following.1) {
                    tracing::warn!("cannot follow the notifications: {error}");
                    thread::sleep(Duration::from_secs(2));
                }
            });
            let serving = (connection.clone(), inbox.outbox());
            thread::spawn(move || serve(&serving.0, &requests, &serving.1));
        }
        report("hide the notifications", window.hide());
        let appearance = window.appearance();
        let mut screen = Screen::new(
            appearance.palette(),
            text,
            Box::new(sender_lookup(entries.clone())),
        );
        screen.set_appearance(
            appearance.palette(),
            appearance.reduce_motion,
            Instant::now(),
        );
        Ok(Self {
            screen,
            inbox,
            outgoing,
            entries,
            region: None,
            keyboard: None,
            hide_pending: false,
            next_refresh: None,
        })
    }

    pub fn toggle(&self) -> impl Fn() + 'static {
        let outbox = self.inbox.outbox();
        move || {
            outbox.send(Message::Toggle);
        }
    }
}
