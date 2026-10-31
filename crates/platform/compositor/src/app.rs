use smithay::reexports::wayland_server::WEnum;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use unbound_protocols::server::unbound_app_v1::{self, UnboundAppV1};
use unbound_protocols::server::unbound_shell_v1::{self, UnboundShellV1};

use crate::shell::{is_shell, toplevel_names};
use crate::state::State;

impl State {
    pub(crate) fn quit_app(&mut self, owner: Option<Client>) {
        let bound: Vec<&UnboundAppV1> = self
            .app_resources
            .iter()
            .filter(|app| app.client() == owner)
            .collect();
        if bound.is_empty() {
            self.windows
                .iter()
                .filter(|window| window.toplevel.wl_surface().client() == owner)
                .for_each(|window| window.toplevel.send_close());
        } else {
            bound.into_iter().for_each(UnboundAppV1::quit_requested);
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
        owners
    }

    pub(crate) fn quit_all_apps(&mut self, requester: UnboundShellV1) {
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

impl GlobalDispatch<UnboundAppV1, ()> for State {
    fn bind(
        state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<UnboundAppV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        state.app_resources.push(data_init.init(resource, ()));
    }
}

impl Dispatch<UnboundAppV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &UnboundAppV1,
        request: unbound_app_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            unbound_app_v1::Request::PlaceWindow {
                toplevel,
                placement: WEnum::Value(placement),
            } if toplevel.client() == resource.client() => {
                state.place_window(&toplevel, placement);
            }
            unbound_app_v1::Request::SetKeepOnTop { toplevel, keep }
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
            unbound_app_v1::Request::MoveToWorkspace {
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
                        workspace == unbound_app_v1::Workspace::Next,
                        false,
                    );
                }
            }
            unbound_app_v1::Request::ShowEmojiPicker => {
                state.open_launcher(unbound_shell_v1::LauncherMode::Emoji);
            }
            unbound_app_v1::Request::Destroy => {
                state.app_resources.retain(|app| app != resource);
            }
            _ => {}
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &UnboundAppV1, _data: &()) {
        state.app_resources.retain(|app| app != resource);
    }
}
