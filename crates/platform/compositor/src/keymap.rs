use std::time::Duration;

use bande_config::{CapsLock, InputSource, Keyboard};
use bande_graphics::TextRole;
use bande_render::VulkanRenderer;
use bande_spring::{Animated, Spring};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::memory::MemoryRenderBufferRenderElement;
use smithay::input::keyboard::{Layout, XkbConfig};
use smithay::utils::{Logical, Point};

use crate::label::{Label, LabelStyle, Look};
use crate::state::{Compositor, OutputElement, State};

const HUD_STYLE: LabelStyle = LabelStyle {
    role: TextRole::Heading,
    padding: (24.0, 12.0),
    look: Look::Card,
};
const HUD_HOLD: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardSettings {
    pub input_sources: Vec<InputSource>,
    pub caps_lock: CapsLock,
    pub repeat_rate: u32,
    pub repeat_delay: Duration,
}

impl KeyboardSettings {
    pub fn resolve(keyboard: &Keyboard, system: impl FnOnce() -> Vec<InputSource>) -> Self {
        Self {
            input_sources: keyboard.input_sources_or(system),
            caps_lock: keyboard.caps_lock,
            repeat_rate: keyboard.repeat_rate,
            repeat_delay: keyboard.repeat_delay,
        }
    }

    fn xkb(&self) -> (String, String, Option<String>) {
        let join = |part: fn(&InputSource) -> &str| {
            self.input_sources
                .iter()
                .map(part)
                .collect::<Vec<_>>()
                .join(",")
        };
        (
            join(|source| &source.layout),
            join(|source| source.variant.as_deref().unwrap_or("")),
            self.caps_lock.xkb_option().map(str::to_owned),
        )
    }
}

struct Hud {
    label: Label,
    screen: usize,
    shown_at: Option<Duration>,
    opacity: f32,
}

#[derive(Default)]
pub(crate) struct InputSources {
    settings: Option<KeyboardSettings>,
    active: usize,
    previous: usize,
    cycling: bool,
    hud: Option<Hud>,
}

fn hud_opacity(elapsed: Duration) -> Option<f32> {
    let mut fade = Animated::new(1.0);
    fade.animate_at(HUD_HOLD, false, 0.0, Spring::FADE);
    let (opacity, fading) = fade.value_at(elapsed);
    fading.then_some(opacity.clamp(0.0, 1.0))
}

impl InputSources {
    pub(crate) fn codes(&self) -> Vec<String> {
        self.settings
            .iter()
            .flat_map(|settings| settings.input_sources.iter().map(InputSource::code))
            .collect()
    }

    pub(crate) fn active(&self) -> usize {
        self.active
    }

    fn count(&self) -> usize {
        self.settings
            .as_ref()
            .map_or(0, |settings| settings.input_sources.len())
    }

    pub(crate) fn is_animating(&self) -> bool {
        self.hud.is_some()
    }

    pub(crate) fn end_cycle(&mut self) {
        self.cycling = false;
    }
}

impl State {
    pub(crate) fn apply_keyboard_settings(&mut self, settings: KeyboardSettings) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let rate = i32::try_from(settings.repeat_rate).unwrap_or(i32::MAX);
        let delay = i32::try_from(settings.repeat_delay.as_millis()).unwrap_or(i32::MAX);
        keyboard.change_repeat_info(rate, delay);
        let sources = &mut self.input_sources;
        let before = sources.settings.as_ref();
        if before.map(KeyboardSettings::xkb) == Some(settings.xkb()) {
            sources.settings = Some(settings);
            return;
        }
        let active = before
            .and_then(|before| before.input_sources.get(sources.active))
            .and_then(|code| {
                settings
                    .input_sources
                    .iter()
                    .position(|known| known == code)
            })
            .unwrap_or(0);
        let (layout, variant, options) = settings.xkb();
        let config = XkbConfig {
            layout: &layout,
            variant: &variant,
            options,
            ..XkbConfig::default()
        };
        if let Err(error) = keyboard.set_xkb_config(self, config) {
            tracing::warn!("cannot use the keyboard layouts {layout}: {error:?}");
            return;
        }
        let sources = &mut self.input_sources;
        sources.settings = Some(settings);
        sources.previous = active;
        self.set_active_source(active);
    }

    fn set_active_source(&mut self, index: usize) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let layout = Layout(u32::try_from(index).unwrap_or(0));
        keyboard.with_xkb_state(self, |mut context| context.set_layout(layout));
        self.input_sources.active = index;
        self.announce_input_sources();
    }

    pub(crate) fn select_input_source(&mut self, index: usize) {
        let sources = &mut self.input_sources;
        if index >= sources.count() || index == sources.active {
            return;
        }
        sources.previous = sources.active;
        self.set_active_source(index);
        self.show_source_hud();
    }

    pub(crate) fn next_input_source(&mut self, backwards: bool) {
        let sources = &mut self.input_sources;
        let count = sources.count();
        if count < 2 {
            return;
        }
        let next = if !sources.cycling && !backwards {
            sources.previous
        } else if backwards {
            (sources.active + count - 1) % count
        } else {
            (sources.active + 1) % count
        };
        let next = if next == sources.active {
            (sources.active + 1) % count
        } else {
            next
        };
        sources.cycling = true;
        sources.previous = sources.active;
        self.set_active_source(next);
        self.show_source_hud();
    }

    fn source_name(&mut self, index: usize) -> Option<String> {
        let keyboard = self.seat.get_keyboard()?;
        keyboard.with_xkb_state(self, |context| {
            let xkb = context.xkb().lock().ok()?;
            Some(
                xkb.layout_name(Layout(u32::try_from(index).ok()?))
                    .to_owned(),
            )
        })
    }

    fn show_source_hud(&mut self) {
        let active = self.input_sources.active;
        let Some(name) = self.source_name(active).filter(|name| !name.is_empty()) else {
            return;
        };
        let screen = self.focused().map_or_else(
            || self.pointer_screen(),
            |focused| self.screen_of_surface(&focused),
        );
        let width = f64::from(self.screen_rect(screen).size.w);
        let scale = self.screens[screen].scale();
        let label =
            self.labels
                .get_mut()
                .render(&name, HUD_STYLE, width, self.decorations.scheme, scale);
        self.input_sources.hud = label.map(|label| Hud {
            label,
            screen,
            shown_at: None,
            opacity: 1.0,
        });
        self.needs_redraw = true;
    }

    pub(crate) fn animate_source_hud(&mut self, time: Duration) {
        let Some(hud) = &mut self.input_sources.hud else {
            return;
        };
        let shown_at = *hud.shown_at.get_or_insert(time);
        match hud_opacity(time.saturating_sub(shown_at)) {
            Some(opacity) => hud.opacity = opacity,
            None => self.input_sources.hud = None,
        }
        self.needs_redraw = true;
    }

    pub(crate) fn source_hud_element(
        &self,
        renderer: &mut VulkanRenderer,
        screen: usize,
        scale: f64,
    ) -> Option<OutputElement> {
        let hud = self
            .input_sources
            .hud
            .as_ref()
            .filter(|hud| hud.screen == screen)?;
        let output = self.screen_rect(screen);
        let location = Point::<f64, Logical>::from((
            f64::from(output.loc.x) + (f64::from(output.size.w) - hud.label.size.w) / 2.0,
            f64::from(output.loc.y) + (f64::from(output.size.h) - hud.label.size.h) / 2.0,
        ))
        .to_physical(scale)
        .to_i32_round::<i32>()
        .to_f64();
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            location,
            &hud.label.buffer,
            Some(hud.opacity),
            None,
            None,
            Kind::Unspecified,
        )
        .ok()
        .map(OutputElement::Memory)
    }
}

impl Compositor {
    pub fn apply_keyboard_settings(&mut self, settings: KeyboardSettings) {
        self.state.apply_keyboard_settings(settings);
        if let Err(error) = self.flush() {
            tracing::warn!("cannot send the keyboard settings: {error}");
        }
    }
}
