use bande_config::{Dragging, Mouse, PrimaryButton, SecondaryClick, Trackpad};
use smithay::backend::input::InputEvent;
use smithay::backend::libinput::LibinputInputBackend;
use smithay::reexports::input::{
    AccelProfile, ClickMethod, Device, DeviceCapability, DeviceConfigResult, ScrollMethod,
};

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

#[derive(Debug, Clone, Default, PartialEq)]
struct Support {
    tap: bool,
    disable_while_typing: bool,
    click_methods: Vec<ClickMethod>,
    natural_scrolling: bool,
    speed: bool,
    accel_profiles: Vec<AccelProfile>,
    left_handed: bool,
    scroll_methods: Vec<ScrollMethod>,
}

impl Support {
    fn read(device: &Device) -> Option<Self> {
        device
            .has_capability(DeviceCapability::Pointer)
            .then(|| Self {
                tap: device.config_tap_finger_count() > 0,
                disable_while_typing: device.config_dwt_is_available(),
                click_methods: device.config_click_methods(),
                natural_scrolling: device.config_scroll_has_natural_scroll(),
                speed: device.config_accel_is_available(),
                accel_profiles: device.config_accel_profiles(),
                left_handed: device.config_left_handed_is_available(),
                scroll_methods: device.config_scroll_methods(),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DeviceOption {
    TapToClick(bool),
    TapDrag(bool),
    DragLock(bool),
    DisableWhileTyping(bool),
    ClickMethod(ClickMethod),
    ScrollMethod(ScrollMethod),
    LeftHanded(bool),
    NaturalScrolling(bool),
    Speed(f64),
    AccelProfile(AccelProfile),
}

fn trackpad_options(support: &Support, trackpad: &Trackpad) -> Vec<DeviceOption> {
    let mut wanted = vec![
        DeviceOption::TapToClick(trackpad.tap_to_click),
        DeviceOption::TapDrag(trackpad.dragging != Dragging::Off),
    ];
    if trackpad.dragging != Dragging::Off {
        wanted.push(DeviceOption::DragLock(
            trackpad.dragging == Dragging::TapWithLock,
        ));
    }
    if support.disable_while_typing {
        wanted.push(DeviceOption::DisableWhileTyping(
            trackpad.disable_while_typing,
        ));
    }
    let click = match trackpad.secondary_click {
        SecondaryClick::TwoFingers => ClickMethod::Clickfinger,
        SecondaryClick::Corner => ClickMethod::ButtonAreas,
    };
    if support.click_methods.contains(&click) {
        wanted.push(DeviceOption::ClickMethod(click));
    }
    let scroll = match trackpad.scroll_method {
        bande_config::ScrollMethod::TwoFingers => ScrollMethod::TwoFinger,
        bande_config::ScrollMethod::Edge => ScrollMethod::Edge,
    };
    if support.scroll_methods.contains(&scroll) {
        wanted.push(DeviceOption::ScrollMethod(scroll));
    }
    wanted
}

fn options(support: &Support, settings: &PointerSettings) -> Vec<DeviceOption> {
    let (mut wanted, natural, speed, acceleration) = if support.tap {
        let trackpad = &settings.trackpad;
        (
            trackpad_options(support, trackpad),
            trackpad.natural_scrolling,
            trackpad.speed,
            trackpad.acceleration,
        )
    } else {
        let mouse = &settings.mouse;
        let left_handed = support
            .left_handed
            .then(|| DeviceOption::LeftHanded(mouse.primary_button == PrimaryButton::Right));
        (
            left_handed.into_iter().collect(),
            mouse.natural_scrolling,
            mouse.speed,
            mouse.acceleration,
        )
    };
    if support.natural_scrolling {
        wanted.push(DeviceOption::NaturalScrolling(natural));
    }
    if support.speed {
        wanted.push(DeviceOption::Speed(speed));
    }
    let profile = if acceleration {
        AccelProfile::Adaptive
    } else {
        AccelProfile::Flat
    };
    if support.accel_profiles.contains(&profile) {
        wanted.push(DeviceOption::AccelProfile(profile));
    }
    wanted
}

fn apply(device: &mut Device, option: DeviceOption) {
    let (setting, result): (&str, DeviceConfigResult) = match option {
        DeviceOption::TapToClick(on) => ("tap to click", device.config_tap_set_enabled(on)),
        DeviceOption::TapDrag(on) => ("tap and drag", device.config_tap_set_drag_enabled(on)),
        DeviceOption::DragLock(on) => ("drag lock", device.config_tap_set_drag_lock_enabled(on)),
        DeviceOption::DisableWhileTyping(on) => {
            ("disable while typing", device.config_dwt_set_enabled(on))
        }
        DeviceOption::ClickMethod(method) => {
            ("the click method", device.config_click_set_method(method))
        }
        DeviceOption::ScrollMethod(method) => {
            ("the scroll method", device.config_scroll_set_method(method))
        }
        DeviceOption::LeftHanded(on) => ("the primary button", device.config_left_handed_set(on)),
        DeviceOption::NaturalScrolling(on) => (
            "natural scrolling",
            device.config_scroll_set_natural_scroll_enabled(on),
        ),
        DeviceOption::Speed(speed) => ("the pointer speed", device.config_accel_set_speed(speed)),
        DeviceOption::AccelProfile(profile) => (
            "pointer acceleration",
            device.config_accel_set_profile(profile),
        ),
    };
    if let Err(error) = result {
        tracing::warn!("cannot set {setting} on {}: {error:?}", device.name());
    }
}

fn configure(device: &mut Device, settings: &PointerSettings) {
    let Some(support) = Support::read(device) else {
        return;
    };
    for option in options(&support, settings) {
        apply(device, option);
    }
}

impl Pointing {
    pub(crate) fn trackpad(&self) -> Trackpad {
        self.settings.trackpad
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

#[cfg(test)]
mod tests {
    use bande_config::{Dragging, PrimaryButton};
    use smithay::reexports::input::{AccelProfile, ClickMethod, ScrollMethod};

    use super::{DeviceOption, PointerSettings, Support, options};

    fn mouse() -> Support {
        Support {
            natural_scrolling: true,
            speed: true,
            accel_profiles: vec![AccelProfile::Flat, AccelProfile::Adaptive],
            left_handed: true,
            scroll_methods: vec![ScrollMethod::NoScroll, ScrollMethod::OnButtonDown],
            ..Support::default()
        }
    }

    fn touchpad() -> Support {
        Support {
            tap: true,
            disable_while_typing: true,
            click_methods: vec![ClickMethod::ButtonAreas, ClickMethod::Clickfinger],
            scroll_methods: vec![ScrollMethod::TwoFinger],
            ..mouse()
        }
    }

    #[test]
    fn a_mouse_gets_its_own_speed_scrolling_primary_button_and_flat_profile() {
        let mut settings = PointerSettings::default();
        settings.mouse.speed = -0.5;
        settings.mouse.natural_scrolling = true;
        settings.mouse.primary_button = PrimaryButton::Right;
        settings.mouse.acceleration = false;
        settings.trackpad.speed = 0.75;
        assert_eq!(
            options(&mouse(), &settings),
            [
                DeviceOption::LeftHanded(true),
                DeviceOption::NaturalScrolling(true),
                DeviceOption::Speed(-0.5),
                DeviceOption::AccelProfile(AccelProfile::Flat),
            ]
        );
    }

    #[test]
    fn a_touchpad_without_edge_scrolling_keeps_its_scroll_method_and_is_never_left_handed() {
        let mut settings = PointerSettings::default();
        settings.trackpad.scroll_method = bande_config::ScrollMethod::Edge;
        settings.mouse.primary_button = PrimaryButton::Right;
        assert_eq!(
            options(&touchpad(), &settings),
            [
                DeviceOption::TapToClick(true),
                DeviceOption::TapDrag(true),
                DeviceOption::DragLock(false),
                DeviceOption::DisableWhileTyping(true),
                DeviceOption::ClickMethod(ClickMethod::Clickfinger),
                DeviceOption::NaturalScrolling(true),
                DeviceOption::Speed(0.0),
                DeviceOption::AccelProfile(AccelProfile::Adaptive),
            ]
        );
        settings.trackpad.scroll_method = bande_config::ScrollMethod::TwoFingers;
        assert!(
            options(&touchpad(), &settings)
                .contains(&DeviceOption::ScrollMethod(ScrollMethod::TwoFinger))
        );
    }

    #[test]
    fn drag_lock_is_set_only_while_dragging_with_lock() {
        let mut settings = PointerSettings::default();
        settings.trackpad.dragging = Dragging::TapWithLock;
        assert!(options(&touchpad(), &settings).contains(&DeviceOption::DragLock(true)));
        settings.trackpad.dragging = Dragging::Off;
        let off = options(&touchpad(), &settings);
        assert!(off.contains(&DeviceOption::TapDrag(false)));
        assert!(
            !off.iter()
                .any(|option| matches!(option, DeviceOption::DragLock(_)))
        );
    }

    #[test]
    fn nothing_is_planned_that_the_device_lacks() {
        assert!(options(&Support::default(), &PointerSettings::default()).is_empty());
        let mut settings = PointerSettings::default();
        settings.trackpad.acceleration = false;
        let adaptive_only = Support {
            accel_profiles: vec![AccelProfile::Adaptive],
            ..touchpad()
        };
        assert!(
            !options(&adaptive_only, &settings)
                .iter()
                .any(|option| matches!(option, DeviceOption::AccelProfile(_)))
        );
    }
}
