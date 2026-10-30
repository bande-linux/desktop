use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New,
};
use unbound_protocols::server::unbound_capture_v1::{self, UnboundCaptureV1};

use crate::shell::is_shell;
use crate::state::State;

impl State {
    pub(crate) fn create_capture_shell_global(display: &DisplayHandle) {
        display.create_global::<State, UnboundCaptureV1, ()>(1, ());
    }

    pub(crate) fn show_capture_options(&mut self) {
        for capture in &self.capture_resources {
            capture.show_options();
        }
    }

    fn list_windows(&self, capture: &UnboundCaptureV1) {
        if !self.is_locked() {
            for window in self
                .windows
                .iter()
                .rev()
                .filter(|window| window.visible() && window.rendered() && self.on_view(window))
            {
                let (Some(handle), Some(geometry)) = (&window.foreign, window.geometry()) else {
                    continue;
                };
                capture.window(
                    handle.identifier(),
                    geometry.location.x,
                    geometry.location.y,
                    geometry.size.w,
                    geometry.size.h,
                );
            }
        }
        capture.windows_done();
    }
}

impl GlobalDispatch<UnboundCaptureV1, ()> for State {
    fn bind(
        state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<UnboundCaptureV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let capture = data_init.init(resource, ());
        state.capture_resources.push(capture);
    }

    fn can_view(client: Client, _global_data: &()) -> bool {
        is_shell(&client)
    }
}

impl Dispatch<UnboundCaptureV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &UnboundCaptureV1,
        request: unbound_capture_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        if let unbound_capture_v1::Request::ListWindows = request {
            state.list_windows(resource);
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, resource: &UnboundCaptureV1, _data: &()) {
        state
            .capture_resources
            .retain(|capture| capture != resource);
    }
}
