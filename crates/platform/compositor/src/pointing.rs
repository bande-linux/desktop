use smithay::backend::input::InputEvent;
use smithay::backend::libinput::LibinputInputBackend;
use smithay::reexports::input::{ClickMethod, Device, DeviceCapability};
use unbound_config::{Mouse, SecondaryClick, Trackpad};

use crate::state::Compositor;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PointerSettings {
    pub trackpad: Trackpad,
    pub mouse: Mouse,
}

#[derive(Default)]
pub(crate) struct Pointing {
    devices: Vec<Device>,
    settings: PointerSettings,
}

fn is_touchpad(device: &Device) -> bool {
    device.config_tap_finger_count() > 0
}

fn configure(device: &mut Device, settings: &PointerSettings) {
    if !device.has_capability(DeviceCapability::Pointer) {
        return;
    }
    let (natural, speed) = if is_touchpad(device) {
        let trackpad = &settings.trackpad;
        let _ = device.config_tap_set_enabled(trackpad.tap_to_click);
        if device.config_dwt_is_available() {
            let _ = device.config_dwt_set_enabled(trackpad.disable_while_typing);
        }
        let method = match trackpad.secondary_click {
            SecondaryClick::TwoFingers => ClickMethod::Clickfinger,
            SecondaryClick::Corner => ClickMethod::ButtonAreas,
        };
        if device.config_click_methods().contains(&method) {
            let _ = device.config_click_set_method(method);
        }
        (trackpad.natural_scrolling, trackpad.speed)
    } else {
        (settings.mouse.natural_scrolling, settings.mouse.speed)
    };
    if device.config_scroll_has_natural_scroll() {
        let _ = device.config_scroll_set_natural_scroll_enabled(natural);
    }
    if device.config_accel_is_available() {
        let _ = device.config_accel_set_speed(speed);
    }
}

impl Compositor {
    pub fn track_libinput_device(&mut self, event: &InputEvent<LibinputInputBackend>) {
        let pointing = &mut self.state.pointing;
        match event {
            InputEvent::DeviceAdded { device } => {
                let mut device = device.clone();
                configure(&mut device, &pointing.settings);
                pointing.devices.push(device);
            }
            InputEvent::DeviceRemoved { device } => {
                pointing.devices.retain(|known| known != device);
            }
            _ => {}
        }
    }

    pub fn apply_pointer_settings(&mut self, settings: PointerSettings) {
        let pointing = &mut self.state.pointing;
        if pointing.settings == settings {
            return;
        }
        pointing.settings = settings;
        for device in &mut pointing.devices {
            configure(device, &settings);
        }
    }
}
