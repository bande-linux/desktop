use std::sync::{Mutex, PoisonError};

use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::{
    Mode as KdeMode, OrgKdeKwinServerDecoration,
};
use smithay::reexports::wayland_server::WEnum;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::kde::decoration::{KdeDecorationHandler, KdeDecorationState};
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::{delegate_kde_decoration, delegate_xdg_decoration};

use crate::state::State;

#[derive(Default)]
struct KdeRequest(Mutex<Option<Mode>>);

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.decorate(&toplevel, Mode::ServerSide);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        self.decorate(&toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.decorate(&toplevel, Mode::ServerSide);
    }
}

impl KdeDecorationHandler for State {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.globals.kde_decoration_state
    }

    fn new_decoration(&mut self, surface: &WlSurface, decoration: &OrgKdeKwinServerDecoration) {
        decoration.mode(KdeMode::Server);
        self.decorate_surface(surface, Mode::ServerSide);
    }

    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &OrgKdeKwinServerDecoration,
        mode: WEnum<KdeMode>,
    ) {
        let WEnum::Value(mode) = mode else {
            return;
        };
        decoration.mode(mode);
        let decided = match mode {
            KdeMode::Server => Mode::ServerSide,
            _ => Mode::ClientSide,
        };
        self.decorate_surface(surface, decided);
    }
}

impl State {
    fn decorate(&mut self, toplevel: &ToplevelSurface, mode: Mode) {
        let surface = toplevel.wl_surface().clone();
        let outer = self
            .window_index(&surface)
            .and_then(|index| self.windows[index].requested_size());
        toplevel.with_pending_state(|state| state.decoration_mode = Some(mode));
        if let Some(index) = self.window_index(&surface)
            && outer.is_some()
        {
            self.windows[index].request_size(outer);
        }
        if toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
        }
    }

    fn decorate_surface(&mut self, surface: &WlSurface, mode: Mode) {
        match self.window_index(surface) {
            Some(index) => {
                let toplevel = self.windows[index].toplevel.clone();
                self.decorate(&toplevel, mode);
            }
            None => with_states(surface, |states| {
                states
                    .data_map
                    .insert_if_missing_threadsafe(KdeRequest::default);
                if let Some(request) = states.data_map.get::<KdeRequest>() {
                    *request.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(mode);
                }
            }),
        }
    }

    pub(crate) fn decorate_as_requested_before(&mut self, toplevel: &ToplevelSurface) {
        let requested = with_states(toplevel.wl_surface(), |states| {
            states
                .data_map
                .get::<KdeRequest>()
                .and_then(|request| *request.0.lock().unwrap_or_else(PoisonError::into_inner))
        });
        if let Some(mode) = requested {
            self.decorate(toplevel, mode);
        }
    }
}

delegate_xdg_decoration!(State);
delegate_kde_decoration!(State);
