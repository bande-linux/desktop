use std::time::Duration;

use smithay::desktop::layer_map_for_output;
use smithay::output::{Mode, Output, PhysicalProperties, Scale as OutputScale, Subpixel};
use smithay::reexports::wayland_server::DisplayHandle;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_callback::WlCallback;
use smithay::utils::{Logical, Physical, Point, Raw, Rectangle, Size, Transform};
use smithay::wayland::presentation::PresentationFeedbackCallback;

use crate::state::{REFRESH, State};
use crate::window::Window;
use crate::workspace::Workspaces;

const REFERENCE_PIXELS_PER_INCH: f64 = 110.0;
const SCALE_STEP: f64 = 0.25;
const HIGH_DENSITY_HEIGHT: i32 = 1600;

pub fn automatic_scale(size: Size<i32, Physical>, physical_size_mm: Size<i32, Raw>) -> f64 {
    if physical_size_mm.w > 0 && physical_size_mm.h > 0 {
        let pixels_per_inch = f64::from(size.w) / (f64::from(physical_size_mm.w) / 25.4);
        let scale = (pixels_per_inch / REFERENCE_PIXELS_PER_INCH / SCALE_STEP).round() * SCALE_STEP;
        return scale.clamp(1.0, 3.0);
    }
    if size.h >= HIGH_DENSITY_HEIGHT {
        2.0
    } else {
        1.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputMode {
    pub size: Size<i32, Physical>,
    pub refresh: Duration,
}

impl OutputMode {
    pub fn millihertz(&self) -> i32 {
        i32::try_from(1_000_000_000_000u128 / self.refresh.as_nanos().max(1)).unwrap_or(i32::MAX)
    }

    fn wayland(&self) -> Mode {
        Mode {
            size: self.size,
            refresh: self.millihertz(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputConfig {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub physical_size_mm: Size<i32, Raw>,
    pub modes: Vec<OutputMode>,
}

impl OutputConfig {
    pub fn virtual_display(name: &str, size: Size<i32, Physical>) -> Self {
        Self {
            name: name.to_owned(),
            make: "Unbound".to_owned(),
            model: "Virtual Display".to_owned(),
            serial: name.to_owned(),
            physical_size_mm: (0, 0).into(),
            modes: vec![OutputMode {
                size,
                refresh: REFRESH,
            }],
        }
    }

    pub fn identity(&self) -> String {
        unbound_edid::identity(&self.make, &self.model, &self.serial)
    }

    pub fn preferred_mode(&self) -> Option<OutputMode> {
        self.modes.first().copied()
    }

    pub fn default_settings(&self) -> Option<OutputSettings> {
        let mode = self.preferred_mode()?;
        Some(OutputSettings {
            mode,
            scale: automatic_scale(mode.size, self.physical_size_mm),
            transform: Transform::Normal,
            position: Point::default(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputSettings {
    pub mode: OutputMode,
    pub scale: f64,
    pub transform: Transform,
    pub position: Point<i32, Logical>,
}

impl OutputSettings {
    pub fn logical_size(&self) -> Size<i32, Logical> {
        self.transform
            .transform_size(self.mode.size)
            .to_f64()
            .to_logical(self.scale)
            .to_i32_round()
    }
}

pub(crate) struct Screen {
    pub(crate) output: Output,
    pub(crate) config: OutputConfig,
    pub(crate) settings: OutputSettings,
    global: GlobalId,
    pub(crate) workspaces: Workspaces,
    pub(crate) redraw: bool,
    pub(crate) mirror: Option<String>,
    pub(crate) latched_feedback: Vec<PresentationFeedbackCallback>,
    pub(crate) latched_frame_callbacks: Vec<WlCallback>,
}

impl Screen {
    pub(crate) fn new(
        display: &DisplayHandle,
        config: OutputConfig,
        settings: OutputSettings,
    ) -> Self {
        let output = Output::new(
            config.name.clone(),
            PhysicalProperties {
                size: config.physical_size_mm,
                subpixel: Subpixel::Unknown,
                make: config.make.clone(),
                model: config.model.clone(),
            },
        );
        for mode in &config.modes {
            output.add_mode(mode.wayland());
        }
        if let Some(preferred) = config.preferred_mode() {
            output.set_preferred(preferred.wayland());
        }
        let global = output.create_global::<State>(display);
        let mut screen = Self {
            output,
            config,
            settings,
            global,
            workspaces: Workspaces::default(),
            redraw: true,
            mirror: None,
            latched_feedback: Vec::new(),
            latched_frame_callbacks: Vec::new(),
        };
        screen.apply(settings);
        screen
    }

    pub(crate) fn apply(&mut self, settings: OutputSettings) {
        self.settings = settings;
        self.output.change_current_state(
            Some(settings.mode.wayland()),
            Some(settings.transform),
            Some(OutputScale::Fractional(settings.scale)),
            Some(settings.position),
        );
        layer_map_for_output(&self.output).arrange();
        self.redraw = true;
    }

    pub(crate) fn retire(&self, display: &DisplayHandle) {
        display.remove_global::<State>(self.global.clone());
    }

    pub(crate) fn name(&self) -> String {
        self.output.name()
    }

    pub(crate) fn scale(&self) -> f64 {
        self.settings.scale
    }

    pub(crate) fn refresh(&self) -> Duration {
        self.settings.mode.refresh
    }

    pub(crate) fn mode_size(&self) -> Size<i32, Physical> {
        self.settings.mode.size
    }

    pub(crate) fn rect(&self) -> Rectangle<i32, Logical> {
        Rectangle::new(self.settings.position, self.settings.logical_size())
    }

    pub(crate) fn contains(&self, point: Point<f64, Logical>) -> bool {
        self.rect().to_f64().contains(point)
    }

    pub(crate) fn usable_area(&self) -> Rectangle<i32, Logical> {
        let zone = layer_map_for_output(&self.output).non_exclusive_zone();
        Rectangle::new(self.settings.position + zone.loc, zone.size)
    }

    pub(crate) fn origin_in_pixels(&self, scale: f64) -> Point<i32, Physical> {
        self.settings
            .position
            .to_f64()
            .to_physical(scale)
            .to_i32_round()
    }
}

impl State {
    pub(crate) fn main_screen(&self) -> &Screen {
        &self.screens[0]
    }

    pub(crate) fn screen_named(&self, name: &str) -> Option<usize> {
        self.screens.iter().position(|screen| screen.name() == name)
    }

    pub(crate) fn screen_at(&self, point: Point<f64, Logical>) -> Option<usize> {
        self.screens
            .iter()
            .position(|screen| screen.mirror.is_none() && screen.contains(point))
    }

    pub(crate) fn pointer_screen(&self) -> usize {
        self.screen_at(self.pointer_location).unwrap_or(0)
    }

    pub(crate) fn screen_of(&self, window: &Window) -> usize {
        self.screen_of_workspace(window.workspace)
    }

    pub(crate) fn screen_of_workspace(&self, workspace: u64) -> usize {
        self.screens
            .iter()
            .position(|screen| screen.workspaces.index_of(workspace).is_some())
            .unwrap_or(0)
    }

    pub(crate) fn screen_rect(&self, screen: usize) -> Rectangle<i32, Logical> {
        self.screens[screen].rect()
    }

    pub(crate) fn usable_area(&self, screen: usize) -> Rectangle<i32, Logical> {
        self.screens[screen].usable_area()
    }

    pub(crate) fn nearest_on_screens(&self, point: Point<f64, Logical>) -> Point<f64, Logical> {
        if self.screen_at(point).is_some() {
            return point;
        }
        self.screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .map(|screen| {
                let rect = screen.rect();
                let clamped: Point<f64, Logical> = (
                    point.x.clamp(
                        f64::from(rect.loc.x),
                        f64::from(rect.loc.x + rect.size.w - 1),
                    ),
                    point.y.clamp(
                        f64::from(rect.loc.y),
                        f64::from(rect.loc.y + rect.size.h - 1),
                    ),
                )
                    .into();
                clamped
            })
            .min_by(|a, b| {
                let distance = |p: &Point<f64, Logical>| (p.x - point.x).hypot(p.y - point.y);
                distance(a).total_cmp(&distance(b))
            })
            .unwrap_or(point)
    }

    pub(crate) fn bounding_rect(&self) -> Rectangle<i32, Logical> {
        self.screens
            .iter()
            .filter(|screen| screen.mirror.is_none())
            .map(Screen::rect)
            .reduce(Rectangle::merge)
            .unwrap_or_default()
    }
}
