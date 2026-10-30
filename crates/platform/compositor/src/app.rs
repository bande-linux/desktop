use smithay::reexports::wayland_server::WEnum;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use unbound_protocols::server::unbound_app_v1::{self, UnboundAppV1};
use unbound_protocols::server::unbound_shell_v1;

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
