use bande_protocols::server::bande_app_control_v1::{self, BandeAppControlV1};
use bande_protocols::server::bande_app_v1::{self, BandeAppV1};
use bande_protocols::server::bande_shell_v1::{self, BandeShellV1};
use smithay::reexports::wayland_server::WEnum;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};

use std::io;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use bande_graphics::Modifiers;
use bande_xdg::{remember_running_apps, write_running_apps};
use smithay::wayland::xdg_activation::XdgActivationTokenData;

use crate::shell::{is_shell, toplevel_names};
use crate::state::{Compositor, State};

const RUNNING_APPS_PAUSE: Duration = Duration::from_secs(1);

pub(crate) struct RunningApps {
    sender: Sender<Vec<String>>,
    written: Vec<String>,
}

impl RunningApps {
    fn start(file: PathBuf) -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel::<Vec<String>>();
        std::thread::Builder::new()
            .name("running-apps".to_owned())
            .spawn(move || {
                while let Ok(mut apps) = receiver.recv() {
                    while let Ok(newer) = receiver.try_recv() {
                        apps = newer;
                    }
                    if let Err(error) = write_running_apps(&file, &apps) {
                        tracing::warn!("cannot note the running apps: {error}");
                    }
                    std::thread::sleep(RUNNING_APPS_PAUSE);
                }
            })?;
        Ok(Self {
            sender,
            written: Vec::new(),
        })
    }
}

impl State {
    pub(crate) fn note_running_apps(&mut self) {
        if !std::mem::take(&mut self.apps_changed) || self.running_apps.is_none() {
            return;
        }
        let mut apps: Vec<String> = Vec::new();
        for (_, app_id) in self.app_owners() {
            if !app_id.is_empty() && !apps.contains(&app_id) {
                apps.push(app_id);
            }
        }
        let Some(running) = self.running_apps.as_mut() else {
            return;
        };
        if running.written == apps {
            return;
        }
        running.written = apps.clone();
        if running.sender.send(apps).is_err() {
            self.running_apps = None;
        }
    }

    pub(crate) fn quit_app(&mut self, owner: Option<Client>) {
        let kept: Vec<&BandeAppV1> = self
            .kept_apps
            .iter()
            .filter(|(app, _)| app.client() == owner)
            .map(|(app, _)| app)
            .collect();
        let bound: Vec<&BandeAppV1> = match kept.is_empty() {
            true => self
                .app_resources
                .iter()
                .filter(|app| app.client() == owner)
                .collect(),
            false => kept,
        };
        if bound.is_empty() {
            self.windows
                .iter()
                .filter(|window| window.toplevel.wl_surface().client() == owner)
                .for_each(|window| window.toplevel.send_close());
        } else {
            bound.into_iter().for_each(BandeAppV1::quit_requested);
        }
    }

    fn app_owners(&self) -> Vec<(Client, String)> {
        let mut owners: Vec<(Client, String)> = Vec::new();
        for window in self.windows.iter().rev() {
            let surface = window.toplevel.wl_surface();
            if let Some(client) = surface.client()
                && !is_shell(&client)
                && !owners.iter().any(|(known, _)| *known == client)
            {
                owners.push((client, toplevel_names(surface).1));
            }
        }
        owners.extend(self.windowless_apps());
        owners
    }

    pub(crate) fn windowless_apps(&self) -> Vec<(Client, String)> {
        let mut found: Vec<(Client, String)> = Vec::new();
        for (resource, app_id) in &self.kept_apps {
            let Some(client) = resource.client() else {
                continue;
            };
            let shown = self
                .windows
                .iter()
                .any(|window| window.toplevel.wl_surface().client().as_ref() == Some(&client));
            if !shown && !is_shell(&client) && !found.iter().any(|(known, _)| *known == client) {
                found.push((client, app_id.clone()));
            }
        }
        found
    }

    pub(crate) fn kept_app_is_active(&self) -> bool {
        self.windowless_app.as_ref().is_some_and(|(client, _)| {
            self.kept_apps
                .iter()
                .any(|(app, _)| app.client().as_ref() == Some(client))
        })
    }

    pub(crate) fn send_app_key(&self, keysym: u32, modifiers: Modifiers) {
        let Some((client, _)) = &self.windowless_app else {
            return;
        };
        let bits = [
            modifiers.shift,
            modifiers.control,
            modifiers.alt,
            modifiers.logo,
        ]
        .into_iter()
        .enumerate()
        .filter(|(_, held)| *held)
        .fold(0, |bits, (bit, _)| bits | 1 << bit);
        for (app, _) in &self.kept_apps {
            if app.client().as_ref() == Some(client) {
                app.key(keysym, bits);
            }
        }
    }

    pub(crate) fn activate_windowless_app(&mut self, app_id: &str) -> Option<Client> {
        let (client, _) = self
            .windowless_apps()
            .into_iter()
            .find(|(_, windowless)| windowless == app_id)?;
        if self.exclusive_layer.is_none() {
            self.focus(None);
            self.windowless_app = Some((client.clone(), app_id.to_owned()));
            self.announce_active_app(None);
        }
        Some(client)
    }

    pub(crate) fn reopen_app(&mut self, app_id: &str) {
        if !self.app_windows(app_id).is_empty() {
            return self.activate_app(app_id);
        }
        let Some(client) = self.activate_windowless_app(app_id) else {
            return;
        };
        let data = XdgActivationTokenData {
            app_id: Some(app_id.to_owned()),
            ..XdgActivationTokenData::default()
        };
        let token = self
            .globals
            .activation_state
            .create_external_token(data)
            .0
            .to_string();
        for (app, _) in &self.kept_apps {
            if app.client().as_ref() == Some(&client) {
                app.reopen(token.clone());
            }
        }
    }

    pub(crate) fn announce_windowless_apps(&mut self) {
        let now: Vec<(String, i32)> = self
            .windowless_apps()
            .into_iter()
            .map(|(client, app_id)| {
                let pid = client
                    .get_credentials(&self.display_handle)
                    .map_or(0, |credentials| credentials.pid);
                (app_id, pid)
            })
            .collect();
        if now == self.announced_windowless {
            return;
        }
        let before = std::mem::replace(&mut self.announced_windowless, now.clone());
        for (app_id, pid) in before.iter().filter(|app| !now.contains(app)) {
            self.each_shell(|shell| shell.windowless_app(app_id.clone(), *pid, 0));
        }
        for (app_id, pid) in now.iter().filter(|app| !before.contains(app)) {
            self.each_shell(|shell| shell.windowless_app(app_id.clone(), *pid, 1));
        }
        self.apps_changed = true;
    }

    pub(crate) fn announce_windowless_apps_to(&self, shell: &BandeShellV1) {
        for (app_id, pid) in &self.announced_windowless {
            shell.windowless_app(app_id.clone(), *pid, 1);
        }
    }

    pub(crate) fn confirm_session_end(&mut self, ending: bande_shell_v1::SessionEnd) {
        self.session_end_requested = Some(ending);
        self.remember_apps_to_reopen(None);
    }

    pub(crate) fn remember_apps_to_reopen(&self, delay: Option<OwnedFd>) {
        let Some(file) = self.reopen_file.clone() else {
            return;
        };
        let running: Vec<String> = self
            .app_owners()
            .into_iter()
            .map(|(_, app_id)| app_id)
            .collect();
        let remembered = std::thread::Builder::new()
            .name("reopen".to_owned())
            .spawn(move || {
                remember_running_apps(&file, &running);
                drop(delay);
            });
        if let Err(error) = remembered {
            tracing::warn!("cannot remember the apps to reopen: {error}");
        }
    }

    pub(crate) fn quit_all_apps(&mut self, requester: BandeShellV1) {
        for (owner, _) in self.app_owners() {
            self.quit_app(Some(owner));
        }
        self.quitting.push((requester, None));
        self.announce_apps_running();
    }

    pub(crate) fn announce_apps_running(&mut self) {
        if self.quitting.is_empty() {
            return;
        }
        let owners = self.app_owners();
        let running = (
            owners
                .first()
                .map(|(_, app_id)| app_id.clone())
                .unwrap_or_default(),
            u32::try_from(owners.len()).unwrap_or(u32::MAX),
        );
        for (shell, announced) in &mut self.quitting {
            if announced.as_ref() != Some(&running) {
                shell.apps_running(running.0.clone(), running.1);
                *announced = Some(running.clone());
            }
        }
    }
}

impl Compositor {
    pub fn remember_apps_to_reopen_in(&mut self, file: PathBuf) {
        self.state.reopen_file = Some(file);
    }

    pub fn note_running_apps_in(&mut self, file: PathBuf) {
        match RunningApps::start(file) {
            Ok(running) => self.state.running_apps = Some(running),
            Err(error) => tracing::warn!("cannot note the running apps: {error}"),
        }
    }

    pub fn shutting_down(&mut self, delay: OwnedFd) {
        let remembered = self
            .state
            .reopen_file
            .as_ref()
            .is_some_and(|file| file.exists());
        if !remembered {
            self.state.remember_apps_to_reopen(Some(delay));
        }
    }
}

impl GlobalDispatch<BandeAppV1, ()> for State {
    fn bind(
        state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<BandeAppV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.app_resources.push(data_init.init(resource, ()));
    }
}

impl Dispatch<BandeAppV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &BandeAppV1,
        request: bande_app_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            bande_app_v1::Request::PlaceWindow {
                toplevel,
                placement: WEnum::Value(placement),
            } if toplevel.client() == resource.client() => {
                state.place_window(&toplevel, placement);
            }
            bande_app_v1::Request::SetKeepOnTop { toplevel, keep }
                if toplevel.client() == resource.client() =>
            {
                if let Some(window) = state
                    .windows
                    .iter_mut()
                    .find(|window| window.toplevel.xdg_toplevel() == &toplevel)
                {
                    window.keep_on_top = keep != 0;
                    state.needs_redraw = true;
                    state.refocus_pointer();
                }
            }
            bande_app_v1::Request::MoveToWorkspace {
                toplevel,
                workspace: WEnum::Value(workspace),
            } if toplevel.client() == resource.client() => {
                if let Some(surface) = state
                    .windows
                    .iter()
                    .find(|window| window.toplevel.xdg_toplevel() == &toplevel)
                    .map(|window| window.surface().clone())
                {
                    state.move_to_workspace(
                        &surface,
                        workspace == bande_app_v1::Workspace::Next,
                        false,
                    );
                }
            }
            bande_app_v1::Request::EndSession {
                ending: WEnum::Value(ending),
            } => state.confirm_session_end(ending),
            bande_app_v1::Request::ShowEmojiPicker => {
                state.open_launcher(bande_shell_v1::LauncherMode::Emoji);
            }
            bande_app_v1::Request::KeepRunning { app_id } => {
                state.kept_apps.retain(|(app, _)| app != resource);
                state.kept_apps.push((resource.clone(), app_id));
            }
            bande_app_v1::Request::Destroy => {
                state.app_resources.retain(|app| app != resource);
                state.kept_apps.retain(|(app, _)| app != resource);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &BandeAppV1, _data: &()) {
        state.app_resources.retain(|app| app != resource);
        state.kept_apps.retain(|(app, _)| app != resource);
    }
}

impl GlobalDispatch<BandeAppControlV1, ()> for State {
    fn bind(
        state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<BandeAppControlV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let control = data_init.init(resource, ());
        state.announce_unresponsive(None, Some(&control));
        state.app_control_resources.push(control);
    }
}

impl Dispatch<BandeAppControlV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &BandeAppControlV1,
        request: bande_app_control_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            bande_app_control_v1::Request::QuitApp { app_id } => {
                for client in state.app_clients(&app_id) {
                    if !is_shell(&client) {
                        state.quit_app(Some(client));
                    }
                }
            }
            bande_app_control_v1::Request::Destroy => {
                state
                    .app_control_resources
                    .retain(|control| control != resource);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &BandeAppControlV1, _data: &()) {
        state
            .app_control_resources
            .retain(|control| control != resource);
    }
}
