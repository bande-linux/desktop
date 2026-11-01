use wayland_client::{Connection, Dispatch, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::decoration::zv1::client::zxdg_decoration_manager_v1::ZxdgDecorationManagerV1;
pub use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use wayland_protocols::xdg::decoration::zv1::client::zxdg_toplevel_decoration_v1::{
    self, ZxdgToplevelDecorationV1,
};

pub use wayland_protocols_misc::server_decoration::client::org_kde_kwin_server_decoration::Mode as KdeMode;
use wayland_protocols_misc::server_decoration::client::org_kde_kwin_server_decoration::{
    self, OrgKdeKwinServerDecoration,
};
pub use wayland_protocols_misc::server_decoration::client::org_kde_kwin_server_decoration_manager::Mode as KdeDefaultMode;
use wayland_protocols_misc::server_decoration::client::org_kde_kwin_server_decoration_manager::{
    self, OrgKdeKwinServerDecorationManager,
};

use crate::{Compositor, Globals, Memory, ShmImage, TestClient};

impl TestClient {
    pub fn show_decorated_window(
        &mut self,
        server: &mut Compositor,
        image: &ShmImage,
        requested: Option<DecorationMode>,
    ) {
        let manager: ZxdgDecorationManagerV1 = self.bind("zxdg_decoration_manager_v1", 1);
        self.map_window(server, image, Memory::Shm, |client, _, toplevel| {
            let decoration = manager.get_toplevel_decoration(toplevel, &client.queue.handle(), ());
            if let Some(mode) = requested {
                decoration.set_mode(mode);
            }
        });
    }

    pub fn decoration_modes(&self) -> &[DecorationMode] {
        &self.globals.decoration_modes
    }
}

impl Dispatch<ZxdgToplevelDecorationV1, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zxdg_toplevel_decoration_v1::Event::Configure {
            mode: WEnum::Value(mode),
        } = event
        {
            globals.decoration_modes.push(mode);
        }
    }
}

delegate_noop!(Globals: ignore ZxdgDecorationManagerV1);

#[derive(Default)]
pub(crate) struct KdeDecoration {
    manager: Option<OrgKdeKwinServerDecorationManager>,
    default_mode: Option<KdeDefaultMode>,
    decorations: Vec<OrgKdeKwinServerDecoration>,
    modes: Vec<KdeMode>,
}

impl TestClient {
    fn kde_manager(&mut self, server: &mut Compositor) -> OrgKdeKwinServerDecorationManager {
        if let Some(manager) = &self.globals.kde_decoration.manager {
            return manager.clone();
        }
        let manager: OrgKdeKwinServerDecorationManager =
            self.bind("org_kde_kwin_server_decoration_manager", 1);
        self.globals.kde_decoration.manager = Some(manager.clone());
        self.exchange(server);
        manager
    }

    pub fn kde_default_mode(&mut self, server: &mut Compositor) -> Option<KdeDefaultMode> {
        self.kde_manager(server);
        self.globals.kde_decoration.default_mode
    }

    pub fn show_kde_decorated_window(
        &mut self,
        server: &mut Compositor,
        image: &ShmImage,
        requested: Option<KdeMode>,
    ) {
        let manager = self.kde_manager(server);
        self.map_window_after(
            server,
            image,
            Memory::Shm,
            |client, surface| {
                let decoration = manager.create(surface, &client.queue.handle(), ());
                if let Some(mode) = requested {
                    decoration.request_mode(mode);
                }
                client.globals.kde_decoration.decorations.push(decoration);
            },
            |_, _, _| {},
        );
    }

    pub fn request_kde_mode(&mut self, server: &mut Compositor, window: usize, mode: KdeMode) {
        self.globals.kde_decoration.decorations[window].request_mode(mode);
        self.exchange(server);
    }

    pub fn kde_modes(&self) -> &[KdeMode] {
        &self.globals.kde_decoration.modes
    }
}

impl Dispatch<OrgKdeKwinServerDecorationManager, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &OrgKdeKwinServerDecorationManager,
        event: org_kde_kwin_server_decoration_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let org_kde_kwin_server_decoration_manager::Event::DefaultMode {
            mode: WEnum::Value(mode),
        } = event
        {
            globals.kde_decoration.default_mode = Some(mode);
        }
    }
}

impl Dispatch<OrgKdeKwinServerDecoration, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &OrgKdeKwinServerDecoration,
        event: org_kde_kwin_server_decoration::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let org_kde_kwin_server_decoration::Event::Mode {
            mode: WEnum::Value(mode),
        } = event
        {
            globals.kde_decoration.modes.push(mode);
        }
    }
}
