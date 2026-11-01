use std::sync::{Arc, Mutex};

use smithay::reexports::wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
};
use smithay::utils::Transform;

use crate::screen::{OutputMode, OutputSettings};
use crate::state::State;

pub(crate) const MANAGER_VERSION: u32 = 4;

pub(crate) struct Head {
    output: String,
    resource: ZwlrOutputHeadV1,
    modes: Vec<(OutputMode, ZwlrOutputModeV1)>,
}

pub(crate) struct Manager {
    resource: ZwlrOutputManagerV1,
    heads: Vec<Head>,
}

type Shared = Arc<Mutex<OutputSettings>>;

#[derive(Default)]
pub(crate) struct Pending {
    serial: u32,
    heads: Mutex<Vec<(String, Option<Shared>)>>,
    used: Mutex<bool>,
}

impl State {
    fn send_head(&self, display: &DisplayHandle, manager: &mut Manager, screen: usize) {
        let Ok(client) = display.get_client(manager.resource.id()) else {
            return;
        };
        let version = manager.resource.version();
        let Ok(resource) = client.create_resource::<ZwlrOutputHeadV1, _, State>(
            display,
            version,
            self.screens[screen].name(),
        ) else {
            return;
        };
        manager.resource.head(&resource);
        let config = &self.screens[screen].config;
        let identity = self.identity_of(screen);
        resource.name(config.name.clone());
        resource.description(identity);
        if config.physical_size_mm.w > 0 && config.physical_size_mm.h > 0 {
            resource.physical_size(config.physical_size_mm.w, config.physical_size_mm.h);
        }
        let mut modes = Vec::new();
        for (index, mode) in config.modes.iter().enumerate() {
            let Ok(announced) = client.create_resource::<ZwlrOutputModeV1, _, State>(
                display,
                version.min(3),
                *mode,
            ) else {
                continue;
            };
            resource.mode(&announced);
            announced.size(mode.size.w, mode.size.h);
            announced.refresh(mode.millihertz());
            if index == 0 {
                announced.preferred();
            }
            modes.push((*mode, announced));
        }
        if version >= zwlr_output_head_v1::EVT_MAKE_SINCE {
            resource.make(config.make.clone());
            resource.model(config.model.clone());
            resource.serial_number(config.serial.clone());
        }
        let mut head = Head {
            output: config.name.clone(),
            resource,
            modes,
        };
        self.send_head_state(&mut head, screen);
        manager.heads.push(head);
    }

    fn send_head_state(&self, head: &mut Head, screen: usize) {
        let settings = self.screens[screen].settings;
        head.resource.enabled(1);
        if let Some((_, mode)) = head.modes.iter().find(|(mode, _)| *mode == settings.mode) {
            head.resource.current_mode(mode);
        }
        head.resource
            .position(settings.position.x, settings.position.y);
        head.resource.transform(settings.transform.into());
        head.resource.scale(settings.scale);
    }

    pub(crate) fn identity_of(&self, screen: usize) -> String {
        let heads: Vec<_> = self
            .screens
            .iter()
            .map(|screen| screen.config.clone())
            .collect();
        crate::arrange::identities(&heads)
            .into_iter()
            .nth(screen)
            .unwrap_or_default()
    }

    pub(crate) fn announce_outputs(&mut self) {
        self.output_serial = self.output_serial.wrapping_add(1);
        let display = self.display_handle.clone();
        let mut managers = std::mem::take(&mut self.output_managers);
        for manager in &mut managers {
            manager.heads.retain(|head| {
                let present = self.screen_named(&head.output).is_some();
                if !present {
                    for (_, mode) in &head.modes {
                        mode.finished();
                    }
                    head.resource.finished();
                }
                present
            });
            for screen in 0..self.screens.len() {
                let name = self.screens[screen].name();
                match manager.heads.iter_mut().find(|head| head.output == name) {
                    Some(head) => self.send_head_state(head, screen),
                    None => self.send_head(&display, manager, screen),
                }
            }
            manager.resource.done(self.output_serial);
        }
        self.output_managers = managers;
    }

    fn apply_configuration(&mut self, pending: &Pending, only_test: bool) -> bool {
        if pending.serial != self.output_serial {
            return false;
        }
        let Ok(heads) = pending.heads.lock() else {
            return false;
        };
        let complete = self.screens.iter().all(|screen| {
            heads
                .iter()
                .any(|(output, settings)| *output == screen.name() && settings.is_some())
        });
        if !complete {
            return false;
        }
        let chosen: Vec<(String, OutputSettings)> = heads
            .iter()
            .filter_map(|(output, settings)| {
                let settings = settings.as_ref()?.lock().ok()?;
                Some((output.clone(), *settings))
            })
            .collect();
        drop(heads);
        if !only_test {
            for (output, settings) in chosen {
                if let Some(screen) = self.screen_named(&output) {
                    self.configure_screen(screen, settings);
                }
            }
            self.announce_outputs();
        }
        true
    }
}

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        display: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let mut manager = Manager {
            resource: data_init.init(resource, ()),
            heads: Vec::new(),
        };
        for screen in 0..state.screens.len() {
            state.send_head(display, &mut manager, screen);
        }
        manager.resource.done(state.output_serial);
        state.output_managers.push(manager);
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwlrOutputManagerV1,
        request: zwlr_output_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_output_manager_v1::Request::CreateConfiguration { id, serial } => {
                data_init.init(
                    id,
                    Pending {
                        serial,
                        ..Pending::default()
                    },
                );
            }
            zwlr_output_manager_v1::Request::Stop => {
                resource.finished();
                state
                    .output_managers
                    .retain(|manager| manager.resource != *resource);
            }
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Self,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &ZwlrOutputManagerV1,
        _data: &(),
    ) {
        state
            .output_managers
            .retain(|manager| manager.resource != *resource);
    }
}

impl Dispatch<ZwlrOutputHeadV1, String> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputHeadV1,
        _request: zwlr_output_head_v1::Request,
        _data: &String,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputModeV1, OutputMode> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputModeV1,
        _request: zwlr_output_mode_v1::Request,
        _data: &OutputMode,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, Pending> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: zwlr_output_configuration_v1::Request,
        pending: &Pending,
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_output_configuration_v1::Request::EnableHead { id, head } => {
                let output = head.data::<String>().cloned().unwrap_or_default();
                let screen = state.screen_named(&output).unwrap_or(0);
                let settings = Arc::new(Mutex::new(state.screens[screen].settings));
                data_init.init(id, settings.clone());
                if let Ok(mut heads) = pending.heads.lock() {
                    heads.push((output, Some(settings)));
                }
            }
            zwlr_output_configuration_v1::Request::DisableHead { head } => {
                let output = head.data::<String>().cloned().unwrap_or_default();
                if let Ok(mut heads) = pending.heads.lock() {
                    heads.push((output, None));
                }
            }
            zwlr_output_configuration_v1::Request::Apply
            | zwlr_output_configuration_v1::Request::Test => {
                let only_test = matches!(request, zwlr_output_configuration_v1::Request::Test);
                if pending
                    .used
                    .lock()
                    .map(|mut used| std::mem::replace(&mut *used, true))
                    .unwrap_or(true)
                {
                    resource.post_error(
                        zwlr_output_configuration_v1::Error::AlreadyUsed,
                        "the configuration was already applied or tested",
                    );
                    return;
                }
                if pending.serial != state.output_serial {
                    resource.cancelled();
                } else if state.apply_configuration(pending, only_test) {
                    resource.succeeded();
                } else {
                    resource.failed();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, Shared> for State {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &ZwlrOutputConfigurationHeadV1,
        request: zwlr_output_configuration_head_v1::Request,
        head: &Shared,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let Ok(mut settings) = head.lock() else {
            return;
        };
        match request {
            zwlr_output_configuration_head_v1::Request::SetMode { mode } => {
                if let Some(mode) = mode.data::<OutputMode>() {
                    settings.mode = *mode;
                }
            }
            zwlr_output_configuration_head_v1::Request::SetPosition { x, y } => {
                settings.position = (x, y).into();
            }
            zwlr_output_configuration_head_v1::Request::SetTransform {
                transform: WEnum::Value(transform),
            } => {
                settings.transform = Transform::from(transform);
            }
            zwlr_output_configuration_head_v1::Request::SetScale { scale }
                if (bande_config::SCALE_RANGE.0..=bande_config::SCALE_RANGE.1).contains(&scale) =>
            {
                settings.scale = scale;
            }
            _ => {}
        }
    }
}
