use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bande_protocols::server::xdg_session_manager_v1::{self, XdgSessionManagerV1};
use bande_protocols::server::xdg_session_v1::{self, XdgSessionV1};
use bande_protocols::server::xdg_toplevel_session_v1::{self, XdgToplevelSessionV1};
use serde::{Deserialize, Serialize};
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::{self, XdgToplevel};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::utils::Rectangle;

use crate::app::StateWriter;
use crate::recall::clamp_into;
use crate::shell::{is_shell, toplevel_names};
use crate::state::{Compositor, State};
use crate::tile::Tile;
use crate::window::Window;

const KEPT_FOR: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const IMPLICIT_PREFIX: &str = "app:";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct Frame {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Placed {
    session: String,
    name: String,
    app_id: String,
    tag: String,
    title: String,
    display: String,
    space: usize,
    frame: Frame,
    tile: Option<Tile>,
    zoomed: bool,
    full_screen: bool,
    minimized: bool,
    rank: usize,
    used: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct Membership {
    session: String,
    name: String,
    restoring: bool,
    handle: XdgToplevelSessionV1,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Resumed {
    minimized: bool,
}

#[derive(Default)]
pub(crate) struct Resume {
    writer: Option<StateWriter>,
    named: Vec<Placed>,
    implicit: Vec<Placed>,
    earlier: Vec<Placed>,
    reopening: Vec<String>,
    live: Vec<(XdgSessionV1, String)>,
    spaces: Vec<(String, usize, u64)>,
    fingerprint: u64,
    frozen: bool,
    pub(crate) ask_after_crash: bool,
}

impl Resume {
    pub(crate) fn freeze(&mut self) {
        self.frozen = true;
    }
}

pub(crate) fn create_global(handle: &DisplayHandle) {
    handle.create_global::<State, XdgSessionManagerV1, ()>(1, ());
}

fn parse(text: &str, now: u64) -> Vec<Placed> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Placed>(line).ok())
        .filter(|placed| placed.used + KEPT_FOR.as_secs() > now)
        .collect()
}

fn format<'a>(records: impl Iterator<Item = &'a Placed>) -> String {
    let mut text = String::new();
    for placed in records {
        match serde_json::to_string(placed) {
            Ok(line) => {
                text.push_str(&line);
                text.push('\n');
            }
            Err(error) => tracing::warn!("cannot note a window: {error}"),
        }
    }
    text
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |time| time.as_secs())
}

fn new_session_id() -> String {
    let mut bytes = [0_u8; 16];
    if let Err(error) = rustix::rand::getrandom(&mut bytes, rustix::rand::GetRandomFlags::empty()) {
        tracing::warn!("no random numbers for a session id: {error}");
    }
    bytes
        .iter()
        .chain(&now().to_le_bytes())
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl State {
    fn placement_fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        for window in self.windows.iter().filter(|window| window.size.is_some()) {
            window.surface().id().protocol_id().hash(&mut hasher);
            let frame = window.committed_frame();
            frame
                .map(|frame| (frame.loc.x, frame.loc.y, frame.size.w, frame.size.h))
                .hash(&mut hasher);
            window
                .restore
                .map(|frame| (frame.loc.x, frame.loc.y, frame.size.w, frame.size.h))
                .hash(&mut hasher);
            window.tile.map(|(tile, _)| tile).hash(&mut hasher);
            window
                .has_state(xdg_toplevel::State::Maximized)
                .hash(&mut hasher);
            window
                .has_state(xdg_toplevel::State::Fullscreen)
                .hash(&mut hasher);
            window.minimized.is_some().hash(&mut hasher);
            let screen = self.screen_of(window);
            screen.hash(&mut hasher);
            self.screens[screen]
                .workspaces
                .index_of(window.workspace)
                .hash(&mut hasher);
            window
                .membership
                .as_ref()
                .map(|membership| &membership.name)
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    fn placed(&self, window: &Window, rank: usize, used: u64) -> Option<Placed> {
        let placed = window.tile.is_some()
            || window.has_state(xdg_toplevel::State::Maximized)
            || window.has_state(xdg_toplevel::State::Fullscreen);
        let frame = placed
            .then_some(window.restore)
            .flatten()
            .or_else(|| window.committed_frame())?;
        let screen = self.screen_of(window);
        let origin = self.screen_rect(screen).loc;
        let (title, app_id) = toplevel_names(window.surface());
        let (session, name) = window.membership.as_ref().map_or_else(
            || (format!("{IMPLICIT_PREFIX}{app_id}"), String::new()),
            |membership| (membership.session.clone(), membership.name.clone()),
        );
        Some(Placed {
            session,
            name,
            app_id,
            tag: window.tag.clone(),
            title,
            display: self.screen_identity(screen),
            space: self.screens[screen]
                .workspaces
                .index_of(window.workspace)
                .unwrap_or(0),
            frame: Frame {
                x: frame.loc.x - origin.x,
                y: frame.loc.y - origin.y,
                width: frame.size.w,
                height: frame.size.h,
            },
            tile: window.tile.map(|(tile, _)| tile),
            zoomed: window.has_state(xdg_toplevel::State::Maximized),
            full_screen: window.has_state(xdg_toplevel::State::Fullscreen),
            minimized: window.minimized.is_some(),
            rank,
            used,
        })
    }

    fn manages_own_session(&self, client: Option<&Client>) -> bool {
        client.is_some_and(|client| {
            self.resume
                .live
                .iter()
                .any(|(session, _)| session.client().as_ref() == Some(client))
        })
    }

    fn implicitly_placed(&self, window: &Window) -> bool {
        let client = window.surface().client();
        window.toplevel.parent().is_none()
            && !window.panel
            && client.as_ref().is_some_and(|client| !is_shell(client))
            && !self.manages_own_session(client.as_ref())
    }

    pub(crate) fn note_window_sessions(&mut self) {
        if self.resume.writer.is_none() || self.resume.frozen || !self.quitting.is_empty() {
            return;
        }
        let fingerprint = self.placement_fingerprint();
        if fingerprint == self.resume.fingerprint {
            return;
        }
        self.resume.fingerprint = fingerprint;
        self.write_window_sessions();
    }

    fn write_window_sessions(&mut self) {
        let used = now();
        let mut implicit = Vec::new();
        for (rank, window) in self.windows.iter().rev().enumerate() {
            if window.size.is_none() {
                continue;
            }
            let Some(mut placed) = self.placed(window, rank, used) else {
                continue;
            };
            if window.membership.is_some() {
                let known = self
                    .resume
                    .named
                    .iter_mut()
                    .find(|known| known.session == placed.session && known.name == placed.name);
                match known {
                    Some(known) => *known = placed,
                    None => self.resume.named.push(placed),
                }
            } else if self.implicitly_placed(window) && !placed.app_id.is_empty() {
                let count = implicit
                    .iter()
                    .filter(|other: &&Placed| {
                        other.app_id == placed.app_id && other.tag == placed.tag
                    })
                    .count();
                placed.name = format!("{}#{count}", placed.tag);
                implicit.push(placed);
            }
        }
        self.resume.implicit = implicit;
        if let Some(writer) = &self.resume.writer {
            writer.write(format(
                self.resume.named.iter().chain(&self.resume.implicit),
            ));
        }
    }

    fn take_implicit_record(&mut self, index: usize) -> Option<Placed> {
        let window = &self.windows[index];
        if !self.implicitly_placed(window) {
            return None;
        }
        let (title, app_id) = toplevel_names(window.surface());
        if !self.resume.reopening.contains(&app_id) {
            return None;
        }
        let tag = window.tag.clone();
        let candidates = |placed: &Placed| placed.app_id == app_id && placed.tag == tag;
        let earlier = &mut self.resume.earlier;
        let at = earlier
            .iter()
            .position(|placed| candidates(placed) && placed.title == title)
            .or_else(|| earlier.iter().position(candidates))?;
        Some(earlier.remove(at))
    }

    pub(crate) fn resume_placement(&mut self, index: usize) -> bool {
        let named = self.windows[index]
            .membership
            .as_ref()
            .filter(|membership| membership.restoring)
            .and_then(|membership| {
                self.resume.named.iter().find(|placed| {
                    placed.session == membership.session && placed.name == membership.name
                })
            })
            .cloned();
        let Some(record) = named.or_else(|| self.take_implicit_record(index)) else {
            return false;
        };
        if let Some(membership) = &self.windows[index].membership {
            membership.handle.restored();
        }
        let screen = (0..self.screens.len())
            .find(|screen| self.screen_identity(*screen) == record.display)
            .unwrap_or_else(|| self.pointer_screen());
        let space = self.space_for_resuming(screen, &record);
        let area = self.usable_area(screen);
        let output = self.screen_rect(screen);
        let frame = clamp_into(
            Rectangle::new(
                (record.frame.x + output.loc.x, record.frame.y + output.loc.y).into(),
                (record.frame.width.max(1), record.frame.height.max(1)).into(),
            ),
            area,
        );
        let window = &mut self.windows[index];
        window.workspace = space;
        window.resumed = Some(Resumed {
            minimized: record.minimized,
        });
        window.resume_rank = Some(record.rank);
        let fixed = window.fixed_size();
        match (record.full_screen, record.zoomed, record.tile) {
            _ if fixed => window.recalled = Some(frame.loc),
            (true, _, _) => {
                self.place(index, xdg_toplevel::State::Fullscreen, Some(output));
                self.windows[index].restore = Some(frame);
            }
            (false, true, _) => {
                self.place(index, xdg_toplevel::State::Maximized, Some(area));
                self.windows[index].restore = Some(frame);
            }
            (false, false, Some(tile)) => {
                let target = tile.rect(area);
                let window = &mut self.windows[index];
                window.restore = Some(frame);
                window.recalled = Some(target.loc);
                window.request_size(Some(target.size));
                self.set_tile(index, Some((tile, target)));
            }
            (false, false, None) => {
                window.recalled = Some(frame.loc);
                window.request_size(Some(frame.size));
            }
        }
        true
    }

    fn space_for_resuming(&mut self, screen: usize, record: &Placed) -> u64 {
        let workspaces = &self.screens[screen].workspaces;
        if record.full_screen {
            return workspaces.active_id();
        }
        let mapped: Vec<(usize, u64)> = self
            .resume
            .spaces
            .iter()
            .filter(|(display, _, id)| {
                *display == record.display && workspaces.index_of(*id).is_some()
            })
            .map(|(_, position, id)| (*position, *id))
            .collect();
        if let Some((_, id)) = mapped
            .iter()
            .find(|(position, _)| *position == record.space)
        {
            return *id;
        }
        let id = if mapped.is_empty() {
            workspaces
                .id_at(0)
                .unwrap_or_else(|| workspaces.active_id())
        } else {
            let at = mapped
                .iter()
                .filter(|(position, _)| *position > record.space)
                .min_by_key(|(position, _)| *position)
                .and_then(|(_, id)| workspaces.index_of(*id))
                .unwrap_or_else(|| workspaces.count());
            let id = self.screens[screen].workspaces.insert(at, None);
            self.announce_workspaces();
            id
        };
        self.resume
            .spaces
            .push((record.display.clone(), record.space, id));
        id
    }

    pub(crate) fn finish_resuming(&mut self, surface: &WlSurface) -> bool {
        let Some(index) = self.window_index(surface) else {
            return false;
        };
        let Some(resumed) = self.windows[index].resumed.take() else {
            return false;
        };
        let rank = self.windows[index].resume_rank.unwrap_or(usize::MAX);
        let above = self.windows.iter().enumerate().position(|(other, window)| {
            other != index
                && window.size.is_some()
                && window.resume_rank.is_some_and(|their| their < rank)
        });
        let in_front = above.is_none();
        if let Some(above) = above.filter(|above| *above < index) {
            let window = self.windows.remove(index);
            self.windows.insert(above, window);
        }
        let focus_is_resumed = self
            .focused()
            .and_then(|focused| self.window_index(&focused))
            .is_none_or(|focused| self.windows[focused].resume_rank.is_some());
        if resumed.minimized {
            self.minimize(surface);
        } else if in_front && focus_is_resumed {
            self.focus(Some(surface.clone()));
            self.show_workspace_of(surface);
        }
        true
    }

    pub(crate) fn note_app_reopening(&mut self, app_id: String) {
        self.resume.reopening.push(app_id);
    }

    fn name_in_use(&self, session: &str, name: &str) -> bool {
        self.windows.iter().any(|window| {
            window
                .membership
                .as_ref()
                .is_some_and(|membership| membership.session == session && membership.name == name)
        })
    }

    fn forget_placement(&mut self, keep: impl Fn(&Placed) -> bool) {
        self.resume.named.retain(keep);
        self.resume.fingerprint = 0;
    }

    fn leave_session(&mut self, session: &str, name: Option<&str>) {
        for window in &mut self.windows {
            if window.membership.as_ref().is_some_and(|membership| {
                membership.session == session && name.is_none_or(|name| membership.name == name)
            }) {
                window.membership = None;
            }
        }
    }

    fn get_session(
        &mut self,
        manager: &XdgSessionManagerV1,
        id: New<XdgSessionV1>,
        wanted: Option<String>,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let client = manager.client();
        let known = wanted.filter(|wanted| {
            self.resume
                .named
                .iter()
                .any(|placed| placed.session == *wanted)
        });
        if let Some(known) = &known
            && let Some((taken, _)) = self
                .resume
                .live
                .iter()
                .find(|(_, session)| session == known)
        {
            if taken.client() == client {
                manager.post_error(
                    xdg_session_manager_v1::Error::InUse,
                    "the session is already in use",
                );
                data_init.init(id, ());
                return;
            }
            taken.replaced();
            let taken = taken.clone();
            self.resume.live.retain(|(session, _)| *session != taken);
            self.leave_session(known, None);
        }
        let session = data_init.init(id, ());
        match known {
            Some(known) => {
                session.restored();
                self.resume.live.push((session, known));
            }
            None => {
                let new = new_session_id();
                session.created(new.clone());
                self.resume.live.push((session, new));
            }
        }
    }

    fn join_session(
        &mut self,
        session: &XdgSessionV1,
        toplevel: &XdgToplevel,
        name: String,
        restoring: bool,
        id: New<XdgToplevelSessionV1>,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let handle = data_init.init(id, ());
        let Some(session_id) = self
            .resume
            .live
            .iter()
            .find(|(live, _)| live == session)
            .map(|(_, session_id)| session_id.clone())
        else {
            return;
        };
        let Some(index) = self
            .windows
            .iter()
            .position(|window| window.toplevel.xdg_toplevel() == toplevel)
        else {
            return;
        };
        let known = self
            .resume
            .named
            .iter()
            .any(|placed| placed.session == session_id && placed.name == name);
        let window = &self.windows[index];
        if window.membership.is_some() {
            session.post_error(
                xdg_session_v1::Error::AlreadyAdded,
                "the toplevel is already in a session",
            );
        } else if self.name_in_use(&session_id, &name) || (known && !restoring) {
            session.post_error(xdg_session_v1::Error::NameInUse, "the name is in use");
        } else if restoring && window.toplevel.is_initial_configure_sent() {
            session.post_error(
                xdg_session_v1::Error::AlreadyMapped,
                "the toplevel has already committed",
            );
        } else {
            self.windows[index].membership = Some(Membership {
                session: session_id,
                name,
                restoring,
                handle,
            });
            self.resume.fingerprint = 0;
        }
    }
}

impl GlobalDispatch<XdgSessionManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<XdgSessionManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<XdgSessionManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        manager: &XdgSessionManagerV1,
        request: xdg_session_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_session_manager_v1::Request::GetSession {
                id,
                reason,
                session_id,
            } => {
                if matches!(reason, WEnum::Unknown(_)) {
                    manager.post_error(
                        xdg_session_manager_v1::Error::InvalidReason,
                        "unknown reason",
                    );
                    data_init.init(id, ());
                    return;
                }
                state.get_session(manager, id, session_id, data_init);
            }
            xdg_session_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

impl Dispatch<XdgSessionV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        session: &XdgSessionV1,
        request: xdg_session_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let session_id = state
            .resume
            .live
            .iter()
            .find(|(live, _)| live == session)
            .map(|(_, session_id)| session_id.clone());
        match request {
            xdg_session_v1::Request::Destroy => {}
            xdg_session_v1::Request::Remove => {
                if let Some(session_id) = session_id {
                    state.forget_placement(|placed| placed.session != session_id);
                    state.leave_session(&session_id, None);
                }
            }
            xdg_session_v1::Request::AddToplevel { id, toplevel, name } => {
                state.join_session(session, &toplevel, name, false, id, data_init);
            }
            xdg_session_v1::Request::RestoreToplevel { id, toplevel, name } => {
                state.join_session(session, &toplevel, name, true, id, data_init);
            }
            xdg_session_v1::Request::RemoveToplevel { name } => {
                if let Some(session_id) = session_id {
                    state.forget_placement(|placed| {
                        placed.session != session_id || placed.name != name
                    });
                    state.leave_session(&session_id, Some(&name));
                }
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        session: &XdgSessionV1,
        _data: &(),
    ) {
        let ended: Vec<String> = state
            .resume
            .live
            .iter()
            .filter(|(live, _)| live == session)
            .map(|(_, session_id)| session_id.clone())
            .collect();
        state.resume.live.retain(|(live, _)| live != session);
        for session_id in ended {
            state.leave_session(&session_id, None);
        }
    }
}

impl Dispatch<XdgToplevelSessionV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        handle: &XdgToplevelSessionV1,
        request: xdg_toplevel_session_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let xdg_toplevel_session_v1::Request::Rename { name } = request {
            let Some(index) = state.windows.iter().position(|window| {
                window
                    .membership
                    .as_ref()
                    .is_some_and(|membership| membership.handle == *handle)
            }) else {
                return;
            };
            let Some(membership) = state.windows[index].membership.clone() else {
                return;
            };
            if state.name_in_use(&membership.session, &name)
                || state
                    .resume
                    .named
                    .iter()
                    .any(|placed| placed.session == membership.session && placed.name == name)
            {
                handle.post_error(xdg_session_v1::Error::NameInUse, "the name is in use");
                return;
            }
            for placed in &mut state.resume.named {
                if placed.session == membership.session && placed.name == membership.name {
                    placed.name.clone_from(&name);
                }
            }
            if let Some(membership) = &mut state.windows[index].membership {
                membership.name = name;
            }
            state.resume.fingerprint = 0;
        }
    }
}

impl Compositor {
    pub fn ask_about_reopening_after_crash(&mut self) {
        self.state.resume.ask_after_crash = true;
    }

    pub fn remember_sessions_in(&mut self, path: PathBuf) {
        let text = fs::read_to_string(&path).unwrap_or_default();
        let (implicit, named): (Vec<Placed>, Vec<Placed>) = parse(&text, now())
            .into_iter()
            .partition(|placed| placed.session.starts_with(IMPLICIT_PREFIX));
        match StateWriter::start(path) {
            Ok(writer) => {
                self.state.resume.writer = Some(writer);
                self.state.resume.named = named;
                self.state.resume.earlier = implicit;
            }
            Err(error) => tracing::warn!("cannot keep the windows' places: {error}"),
        }
    }
}
