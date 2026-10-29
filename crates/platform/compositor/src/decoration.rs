use smithay::delegate_xdg_decoration;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;

use crate::state::State;

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        decorate_client_side(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        decorate_client_side(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        decorate_client_side(&toplevel);
    }
}

fn decorate_client_side(toplevel: &ToplevelSurface) {
    toplevel.with_pending_state(|state| state.decoration_mode = Some(Mode::ClientSide));
    if toplevel.is_initial_configure_sent() {
        toplevel.send_configure();
    }
}

delegate_xdg_decoration!(State);
