use smithay::delegate_xdg_decoration;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;

use crate::state::State;

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
}

delegate_xdg_decoration!(State);
