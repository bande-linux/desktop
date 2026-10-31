use std::collections::HashMap;

use zbus::zvariant::{DeserializeDict, OwnedValue, Type};

pub const BUS_NAME: &str = "org.unbound.Config";
pub const OBJECT_PATH: &str = "/org/unbound/Config";

pub const INTERFACE: &str = "org.unbound.Config";

pub type ArrangedDisplay = (String, i32, i32, bool, String);
pub type NightLight = (bool, String, String, String, u32);
pub type Displays = HashMap<String, HashMap<String, OwnedValue>>;

macro_rules! config_interface {
    ($($name:ident, $set:ident: $read:ty, $write:ty;)*) => {
        #[zbus::proxy(
            interface = "org.unbound.Config",
            default_service = "org.unbound.Config",
            default_path = "/org/unbound/Config",
            gen_async = false,
            blocking_name = "ConfigProxy"
        )]
        pub trait Config {
            $(
                #[zbus(property)]
                fn $name(&self) -> zbus::Result<$read>;
                #[zbus(property)]
                fn $set(&self, value: $write) -> zbus::Result<()>;
            )*
            #[zbus(property)]
            fn night_light(&self) -> zbus::Result<NightLight>;
            #[zbus(property)]
            fn displays(&self) -> zbus::Result<Displays>;
            fn set_display(
                &self,
                identity: &str,
                settings: HashMap<&str, zbus::zvariant::Value<'_>>,
            ) -> zbus::Result<()>;
            #[zbus(property)]
            fn arrangements(&self) -> zbus::Result<Vec<Vec<ArrangedDisplay>>>;
            fn set_arrangement(&self, displays: Vec<ArrangedDisplay>) -> zbus::Result<()>;
        }

        #[derive(Debug, Default, PartialEq, DeserializeDict, Type)]
        #[zvariant(signature = "a{sv}", rename_all = "PascalCase", crate = "zbus::zvariant")]
        pub struct Properties {
            $(pub $name: Option<$read>,)*
            pub night_light: Option<NightLight>,
            pub displays: Option<Displays>,
            pub arrangements: Option<Vec<Vec<ArrangedDisplay>>>,
        }

        impl Properties {
            pub fn update(&mut self, newer: Self) {
                $(
                    if newer.$name.is_some() {
                        self.$name = newer.$name;
                    }
                )*
                if newer.night_light.is_some() {
                    self.night_light = newer.night_light;
                }
                if newer.displays.is_some() {
                    self.displays = newer.displays;
                }
                if newer.arrangements.is_some() {
                    self.arrangements = newer.arrangements;
                }
            }
        }
    };
}

config_interface! {
    appearance, set_appearance: String, &str;
    accent, set_accent: String, &str;
    tinted_frame, set_tinted_frame: bool, bool;
    clock_format, set_clock_format: String, &str;
    show_battery_percentage, set_show_battery_percentage: bool, bool;
    low_power_on_battery, set_low_power_on_battery: bool, bool;
    lid_on_power, set_lid_on_power: String, &str;
    screen_reader, set_screen_reader: bool, bool;
    zoom, set_zoom: f64, f64;
    increase_contrast, set_increase_contrast: bool, bool;
    reduce_transparency, set_reduce_transparency: bool, bool;
    reduce_motion, set_reduce_motion: bool, bool;
    text_size, set_text_size: u32, u32;
    cursor_size, set_cursor_size: u32, u32;
    flash_screen, set_flash_screen: bool, bool;
    sticky_keys, set_sticky_keys: bool, bool;
    slow_keys, set_slow_keys: bool, bool;
    bounce_keys, set_bounce_keys: bool, bool;
    bar_apps, set_bar_apps: Vec<String>, &[String];
    bar_auto_hide, set_bar_auto_hide: bool, bool;
    bar_icon_size, set_bar_icon_size: u32, u32;
    bar_names, set_bar_names: bool, bool;
    bar_edge, set_bar_edge: String, &str;
    bar_menu_bar, set_bar_menu_bar: String, &str;
    idle_dim_after, set_idle_dim_after: u64, u64;
    idle_lock_after, set_idle_lock_after: u64, u64;
    idle_suspend_after, set_idle_suspend_after: u64, u64;
    idle_suspend_with_display, set_idle_suspend_with_display: bool, bool;
    key_repeat_rate, set_key_repeat_rate: u32, u32;
    key_repeat_delay, set_key_repeat_delay: u32, u32;
    input_sources, set_input_sources: Vec<String>, &[String];
    caps_lock, set_caps_lock: String, &str;
    trackpad_tap_to_click, set_trackpad_tap_to_click: bool, bool;
    trackpad_natural_scrolling, set_trackpad_natural_scrolling: bool, bool;
    trackpad_speed, set_trackpad_speed: f64, f64;
    trackpad_secondary_click, set_trackpad_secondary_click: String, &str;
    trackpad_disable_while_typing, set_trackpad_disable_while_typing: bool, bool;
    mouse_natural_scrolling, set_mouse_natural_scrolling: bool, bool;
    mouse_speed, set_mouse_speed: f64, f64;
    sound_effects, set_sound_effects: bool, bool;
    volume_feedback, set_volume_feedback: bool, bool;
    alert_sound, set_alert_sound: String, &str;
    alert_volume, set_alert_volume: f64, f64;
    wallpaper_light, set_wallpaper_light: String, &str;
    wallpaper_dark, set_wallpaper_dark: String, &str;
    wallpaper_fit, set_wallpaper_fit: String, &str;
}
