use smithay::reexports::wayland_protocols_wlr::output_power_management::v1::server::{
    zwlr_output_power_manager_v1::{self, ZwlrOutputPowerManagerV1},
    zwlr_output_power_v1::{self, Mode, ZwlrOutputPowerV1},
};
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};

use crate::state::{Compositor, State};

const MANAGER_VERSION: u32 = 1;

#[derive(Default)]
pub(crate) struct OutputPower {
    controls: Vec<(ZwlrOutputPowerV1, String)>,
    off: Vec<String>,
    turned_on: bool,
}

pub(crate) fn create_global(display: &DisplayHandle) {
    display.create_global::<State, ZwlrOutputPowerManagerV1, ()>(MANAGER_VERSION, ());
}

impl State {
    fn power_mode(&self, output: &str) -> Mode {
        if self.output_power.off.iter().any(|name| name == output) {
            Mode::Off
        } else {
            Mode::On
        }
    }

    fn output_name(&self, output: &WlOutput) -> Option<String> {
        self.screens
            .iter()
            .find(|screen| screen.output.owns(output))
            .map(|screen| screen.name())
    }

    fn set_output_power(&mut self, output: &str, mode: Mode) {
        if self.power_mode(output) == mode {
            return;
        }
        match mode {
            Mode::Off => {
                self.output_power.off.push(output.to_owned());
                self.settle_lock();
            }
            _ => {
                self.output_power.off.retain(|name| name != output);
                self.output_power.turned_on = true;
            }
        }
        self.needs_redraw = true;
        self.output_power
            .controls
            .retain(|(control, _)| control.is_alive());
        for (control, _) in self
            .output_power
            .controls
            .iter()
            .filter(|(_, name)| name == output)
        {
            control.mode(mode);
        }
    }

    pub(crate) fn powered(&self, output: &str) -> bool {
        self.power_mode(output) == Mode::On
    }

    pub(crate) fn power_on_every_output(&mut self) {
        for output in self.output_power.off.clone() {
            self.set_output_power(&output, Mode::On);
        }
    }
}

impl Compositor {
    pub fn output_powered(&self, output: &str) -> bool {
        self.state.powered(output)
    }

    pub fn take_outputs_powered_on(&mut self) -> bool {
        std::mem::take(&mut self.state.output_power.turned_on)
    }
}

impl GlobalDispatch<ZwlrOutputPowerManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputPowerManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrOutputPowerManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &ZwlrOutputPowerManagerV1,
        request: zwlr_output_power_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let zwlr_output_power_manager_v1::Request::GetOutputPower { id, output } = request else {
            return;
        };
        let control = data_init.init(id, ());
        match state.output_name(&output) {
            Some(name) => {
                control.mode(state.power_mode(&name));
                state.output_power.controls.push((control, name));
            }
            None => control.failed(),
        }
    }
}

impl Dispatch<ZwlrOutputPowerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        control: &ZwlrOutputPowerV1,
        request: zwlr_output_power_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let output = state
            .output_power
            .controls
            .iter()
            .find(|(known, _)| known == control)
            .map(|(_, name)| name.clone());
        match request {
            zwlr_output_power_v1::Request::SetMode {
                mode: WEnum::Value(mode),
            } => match output.filter(|name| state.screen_named(name).is_some()) {
                Some(name) => state.set_output_power(&name, mode),
                None => control.failed(),
            },
            zwlr_output_power_v1::Request::Destroy => {
                state
                    .output_power
                    .controls
                    .retain(|(known, _)| known != control);
            }
            _ => {}
        }
    }
}
