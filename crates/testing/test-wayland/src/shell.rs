use bande_compositor::SessionRole;
use bande_protocols::client::bande_shell_v1::BandeShellV1;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};

use crate::{Compositor, Globals, TestClient};

impl TestClient {
    pub fn connect_as_shell(server: &mut Compositor) -> Self {
        let (client_end, server_end) = std::os::unix::net::UnixStream::pair().expect("socket pair");
        server
            .insert_session_client(server_end, SessionRole::Shell)
            .expect("insert shell client");
        Self::from_socket(server, client_end)
    }

    pub fn join_shell(&mut self, server: &mut Compositor) {
        let _: BandeShellV1 = self.bind("bande_shell_v1", 1);
        let _: ExtForeignToplevelListV1 = self.bind("ext_foreign_toplevel_list_v1", 1);
        self.exchange(server);
    }

    pub fn set_app_id(&mut self, server: &mut Compositor, index: usize, app_id: &str) {
        self.windows[index].2.set_app_id(app_id.to_owned());
        self.exchange(server);
    }
}

delegate_noop!(Globals: ignore BandeShellV1);
delegate_noop!(Globals: ignore ExtForeignToplevelHandleV1);

impl Dispatch<ExtForeignToplevelListV1, ()> for Globals {
    fn event(
        _: &mut Self,
        _: &ExtForeignToplevelListV1,
        _: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(Globals, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}
