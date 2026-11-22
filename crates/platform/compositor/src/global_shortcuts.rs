use std::io;
use std::path::Path;
use std::sync::mpsc::{Sender, channel};
use std::thread;

use bande_keyboard::{GlobalShortcut, Key, Modifiers, Trigger, read_global_shortcuts};
use bande_watch::{Changes, FolderWatch};
use zbus::blocking::Connection;
use zbus::blocking::fdo::DBusProxy;
use zbus::names::WellKnownName;

use crate::state::Compositor;

const NAME: &str = "org.bande.GlobalShortcuts";
const PATH: &str = "/org/bande/GlobalShortcuts";
const PORTAL: &str = "org.freedesktop.impl.portal.desktop.bande";

struct Pressed {
    app_id: String,
    id: String,
    time: u32,
    activated: bool,
}

#[derive(Default)]
pub(crate) struct GlobalShortcuts {
    bound: Vec<GlobalShortcut>,
    held: Vec<(u32, String, String)>,
    handle: Option<GlobalShortcutsHandle>,
}

impl GlobalShortcuts {
    fn send(&self, app_id: String, id: String, time: u32, activated: bool) {
        if let Some(handle) = &self.handle {
            let _ = handle.events.send(Pressed {
                app_id,
                id,
                time,
                activated,
            });
        }
    }

    pub(crate) fn press(&mut self, code: u32, modifiers: Modifiers, key: Key, time: u32) -> bool {
        let Some(shortcut) = self
            .bound
            .iter()
            .find(|shortcut| shortcut.trigger == Some(Trigger { modifiers, key }))
        else {
            return false;
        };
        let (app_id, id) = (shortcut.app_id.clone(), shortcut.id.clone());
        self.held.push((code, app_id.clone(), id.clone()));
        self.send(app_id, id, time, true);
        true
    }

    pub(crate) fn release(&mut self, code: u32, time: u32) {
        if let Some(index) = self.held.iter().position(|(held, ..)| *held == code) {
            let (_, app_id, id) = self.held.remove(index);
            self.send(app_id, id, time, false);
        }
    }
}

fn read_bound(file: &Path) -> Vec<GlobalShortcut> {
    match std::fs::read_to_string(file) {
        Ok(text) => read_global_shortcuts(&text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            tracing::warn!("cannot read the global shortcuts: {error}");
            Vec::new()
        }
    }
}

fn announce(connection: &Connection, pressed: &Pressed) -> zbus::Result<()> {
    let portal = DBusProxy::new(connection)?
        .get_name_owner(WellKnownName::from_static_str_unchecked(PORTAL).into())?;
    connection.emit_signal(
        Some(&portal),
        PATH,
        NAME,
        if pressed.activated {
            "Activated"
        } else {
            "Deactivated"
        },
        &(
            pressed.app_id.as_str(),
            pressed.id.as_str(),
            u64::from(pressed.time),
        ),
    )
}

pub struct GlobalShortcutsHandle {
    events: Sender<Pressed>,
    _watch: FolderWatch,
}

pub fn serve_global_shortcuts(
    connection: &Connection,
    mut publish: impl FnMut(Vec<GlobalShortcut>) -> bool + Send + 'static,
) -> io::Result<GlobalShortcutsHandle> {
    let file = bande_xdg::global_shortcuts_file()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "there is no state folder"))?;
    let folder = file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "there is no state folder"))?;
    std::fs::create_dir_all(&folder)?;
    publish(read_bound(&file));
    let watched = file.clone();
    let watch = FolderWatch::spawn(Changes::Saved, move |changed| {
        if changed.contains(&watched) {
            publish(read_bound(&watched));
        }
    })?;
    watch.watch(&folder)?;
    connection.request_name(NAME).map_err(io::Error::other)?;
    let (events, received) = channel::<Pressed>();
    let emitting = connection.clone();
    thread::Builder::new()
        .name("global-shortcuts".to_owned())
        .spawn(move || {
            for pressed in received {
                if let Err(error) = announce(&emitting, &pressed) {
                    tracing::debug!("cannot pass a global shortcut to the portal: {error}");
                }
            }
        })?;
    Ok(GlobalShortcutsHandle {
        events,
        _watch: watch,
    })
}

impl Compositor {
    pub fn use_global_shortcuts(&mut self, handle: GlobalShortcutsHandle) {
        self.state.global_shortcuts.handle = Some(handle);
    }

    pub fn bind_global_shortcuts(&mut self, bound: Vec<GlobalShortcut>) {
        self.state.global_shortcuts.bound = bound;
    }
}
