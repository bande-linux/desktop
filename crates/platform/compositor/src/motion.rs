use std::time::Duration;

use smithay::desktop::layer_map_for_output;
use smithay::utils::{Logical, Physical, Point, Rectangle, Size};
use unbound_spring::{Animated, Spring, State as Phase};

use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

use crate::scene::Closing;
use crate::screen::Screen;
use crate::shell::toplevel_names;
use crate::state::{Compositor, State};
use crate::window::Window;

const SETTLED_PIXELS: f64 = 0.25;
pub(crate) const SETTLED_ALPHA: f64 = 1.0 / 512.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Tick {
    pub(crate) time: Duration,
    pub(crate) refresh: Duration,
    pub(crate) scale: f64,
}

impl Tick {
    fn lead(&self) -> Duration {
        self.time + self.refresh
    }

    pub(crate) fn drive(&self, animated: &mut Animated<Duration>) -> Duration {
        animated.start_at(self.time);
        self.lead()
    }
}

pub(crate) fn progress_precision(span: Size<i32, Physical>) -> f64 {
    SETTLED_PIXELS / f64::from(span.w.max(span.h).max(1))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Look {
    pub(crate) rect: Rectangle<f64, Logical>,
    pub(crate) alpha: f64,
}

impl Look {
    pub(crate) fn shown(rect: Rectangle<f64, Logical>) -> Self {
        Self { rect, alpha: 1.0 }
    }

    pub(crate) fn hidden(rect: Rectangle<f64, Logical>) -> Self {
        Self { rect, alpha: 0.0 }
    }

    pub(crate) fn scaled(self, factor: f64) -> Self {
        let size = self.rect.size.upscale(factor);
        let center = self.rect.loc + self.rect.size.downscale(2.0).to_point();
        Self {
            rect: Rectangle::new(center - size.downscale(2.0).to_point(), size),
            alpha: self.alpha,
        }
    }

    fn values(self) -> [f64; 5] {
        [
            self.rect.loc.x,
            self.rect.loc.y,
            self.rect.size.w,
            self.rect.size.h,
            self.alpha,
        ]
    }

    fn from_values([x, y, w, h, alpha]: [f64; 5]) -> Self {
        Self {
            rect: Rectangle::new((x, y).into(), (w.max(0.0), h.max(0.0)).into()),
            alpha: alpha.clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Motion {
    spring: Spring,
    channels: [Animated<Duration>; 5],
    targets: [f64; 5],
    now: Option<Duration>,
    at_rest: bool,
}

fn still(value: f64) -> Phase {
    Phase {
        position: value,
        velocity: 0.0,
    }
}

impl Motion {
    pub(crate) fn new(spring: Spring, from: Look, to: Look) -> Self {
        let (from, targets) = (from.values(), to.values());
        Self {
            spring,
            channels: std::array::from_fn(|index| {
                Animated::released(still(from[index]), targets[index], spring)
            }),
            targets,
            now: None,
            at_rest: false,
        }
    }

    fn state(&self, index: usize) -> Phase {
        self.channels[index].state_at(self.now.unwrap_or_default())
    }

    pub(crate) fn look(&self) -> Look {
        Look::from_values(std::array::from_fn(|index| self.state(index).position))
    }

    pub(crate) fn shown(&self) -> Option<Look> {
        self.now.map(|_| self.look())
    }

    fn restart(&mut self, index: usize, from: Phase) {
        self.channels[index] = Animated::released(from, self.targets[index], self.spring);
        if let Some(now) = self.now {
            self.channels[index].start_at(now);
        }
        self.at_rest = false;
    }

    pub(crate) fn retarget(&mut self, to: Look) {
        for (index, target) in to.values().into_iter().enumerate() {
            if self.targets[index] != target {
                self.targets[index] = target;
                self.restart(index, self.state(index));
            }
        }
    }

    pub(crate) fn translate(&mut self, dx: f64) {
        let state = self.state(0);
        self.restart(
            0,
            Phase {
                position: state.position + dx,
                velocity: state.velocity,
            },
        );
    }

    pub(crate) fn advance(&mut self, tick: Tick) {
        for channel in &mut self.channels {
            channel.start_at(tick.time);
        }
        let now = tick.lead();
        self.now = Some(now);
        let [x, y, w, h, alpha] = &self.channels;
        let pixel = SETTLED_PIXELS / tick.scale;
        self.at_rest = [x, y, w, h]
            .iter()
            .all(|channel| channel.is_at_rest(now, pixel))
            && alpha.is_at_rest(now, SETTLED_ALPHA);
    }

    pub(crate) fn place(&mut self, rect: Rectangle<f64, Logical>) {
        let values = Look::shown(rect).values();
        for (index, value) in values.into_iter().enumerate().take(4) {
            self.targets[index] = value;
            self.channels[index] = Animated::released(still(value), value, Spring::STANDARD);
        }
    }

    pub(crate) fn is_settled(&self) -> bool {
        self.at_rest
    }
}

pub(crate) struct IconRect {
    app_id: String,
    surface: WlSurface,
    rect: Rectangle<i32, Logical>,
}

pub(crate) const OPENING_SCALE: f64 = 0.96;
const CLOSING_SCALE: f64 = 0.98;
const DOCK_ICON_SIZE: f64 = 48.0;
pub(crate) const DOCK_NAMESPACE: &str = "dock";
pub(crate) const BAR_NAMESPACE: &str = "bar";

impl State {
    fn rect_of(window: &Window) -> Option<Rectangle<f64, Logical>> {
        let geometry = window.geometry()?;
        let (location, size) = window
            .pending_location
            .map(|(_, location)| {
                let size = window.requested_size().unwrap_or(geometry.size);
                (location, size)
            })
            .unwrap_or((geometry.loc, geometry.size));
        Some(Rectangle::new(location.to_f64(), size.to_f64()))
    }

    fn destination(&self, window: &Window) -> Option<Look> {
        let rect = Self::rect_of(window)?;
        if window.hidden {
            return Some(Look::hidden(rect));
        }
        if window.minimized.is_some() {
            return Some(if self.reduce_motion {
                Look::hidden(rect)
            } else {
                Look::shown(self.dock_target(window, rect))
            });
        }
        Some(Look::shown(rect))
    }

    pub(crate) fn set_icon_rect(
        &mut self,
        app_id: String,
        surface: WlSurface,
        rect: Rectangle<i32, Logical>,
    ) {
        self.icon_rects
            .retain(|icon| icon.app_id != app_id && icon.surface.is_alive());
        if !rect.is_empty() {
            self.icon_rects.push(IconRect {
                app_id,
                surface,
                rect,
            });
        }
    }

    fn icon_of(&self, window: &Window) -> Option<Rectangle<f64, Logical>> {
        let app_id = toplevel_names(window.surface()).1;
        let icon = self
            .icon_rects
            .iter()
            .find(|icon| icon.app_id == app_id && icon.surface.is_alive())?;
        let origin = self.layer_origin(&icon.surface)?;
        Some(Rectangle::new(origin + icon.rect.loc, icon.rect.size).to_f64())
    }

    fn dock_target(
        &self,
        window: &Window,
        rect: Rectangle<f64, Logical>,
    ) -> Rectangle<f64, Logical> {
        if let Some(icon) = self.icon_of(window) {
            let factor =
                (icon.size.w / rect.size.w.max(1.0)).min(icon.size.h / rect.size.h.max(1.0));
            let size = rect.size.upscale(factor);
            let center = icon.loc + icon.size.downscale(2.0).to_point();
            return Rectangle::new(center - size.downscale(2.0).to_point(), size);
        }
        let dock = self.screens.iter().find_map(|screen| {
            let map = layer_map_for_output(&screen.output);
            map.layers()
                .find(|layer| [DOCK_NAMESPACE, BAR_NAMESPACE].contains(&layer.namespace()))
                .and_then(|layer| map.layer_geometry(layer))
                .map(|geometry| (screen.settings.position + geometry.loc, geometry.size))
        });
        let center: Point<f64, Logical> = match dock {
            Some((at, size)) => (at + size.downscale(2).to_point()).to_f64(),
            None => {
                let output = self.screen_rect(self.screen_of(window));
                let origin = output.loc.to_f64();
                (
                    origin.x + f64::from(output.size.w) / 2.0,
                    origin.y + f64::from(output.size.h),
                )
                    .into()
            }
        };
        let factor = (DOCK_ICON_SIZE / rect.size.w.max(rect.size.h).max(1.0)).min(1.0);
        let size = rect.size.upscale(factor);
        Rectangle::new(center - size.downscale(2.0).to_point(), size)
    }

    pub(crate) fn animate_window(&mut self, index: usize, spring: Spring, from: Option<Look>) {
        self.needs_redraw = true;
        let window = &self.windows[index];
        if window.motion.is_some() {
            return;
        }
        let (Some(target), Some(rect)) = (self.destination(window), window.geometry()) else {
            return;
        };
        let real = Look::shown(rect.to_f64());
        let mut from = from.unwrap_or(real);
        let spring = if self.reduce_motion {
            from.rect = target.rect;
            Spring::FADE
        } else {
            spring
        };
        self.windows[index].motion = Some(Motion::new(spring, from, target));
    }

    pub(crate) fn open_window(&mut self, index: usize) {
        let Some(rect) = self.windows[index].geometry() else {
            return;
        };
        let real = rect.to_f64();
        self.windows[index].motion = None;
        self.animate_window(
            index,
            Spring::STANDARD,
            Some(Look::hidden(real).scaled(OPENING_SCALE)),
        );
    }

    pub(crate) fn restore_window(&mut self, index: usize) {
        let from = self.windows[index]
            .geometry()
            .map(|rect| rect.to_f64())
            .map(|rect| {
                if self.reduce_motion {
                    Look::hidden(rect)
                } else {
                    Look::shown(self.dock_target(&self.windows[index], rect))
                }
            });
        self.animate_window(index, Spring::GENTLE, from);
    }

    pub(crate) fn close_window(&mut self, index: usize) {
        let output = self.screens[self.screen_of(&self.windows[index])].name();
        let window = &mut self.windows[index];
        window.motion = None;
        let Some(drawn) = window.drawn.take() else {
            return;
        };
        let from = drawn.look;
        let (to, spring) = if self.reduce_motion {
            (Look::hidden(from.rect), Spring::FADE)
        } else {
            (Look::hidden(from.rect).scaled(CLOSING_SCALE), Spring::QUICK)
        };
        self.closing
            .push(Closing::new(drawn, Motion::new(spring, from, to), output));
        self.needs_redraw = true;
    }

    fn tick(&self, time: Duration, screen: usize) -> Tick {
        Tick {
            time,
            refresh: self.screens[screen].refresh(),
            scale: self.screens.iter().map(Screen::scale).fold(1.0, f64::max),
        }
    }

    pub(crate) fn progress_precision(&self) -> f64 {
        self.screens
            .iter()
            .map(|screen| progress_precision(screen.mode_size()))
            .fold(f64::INFINITY, f64::min)
    }

    fn animate(&mut self, tick: Tick) {
        self.keep_families_together();
        let reduce = self.reduce_motion;
        for index in 0..self.windows.len() {
            if self.windows[index].motion.is_none() {
                continue;
            }
            let target = self.destination(&self.windows[index]);
            let window = &mut self.windows[index];
            let (Some(motion), Some(target)) = (&mut window.motion, target) else {
                window.motion = None;
                continue;
            };
            if reduce {
                motion.place(target.rect);
            }
            motion.retarget(target);
            motion.advance(tick);
            if motion.is_settled() {
                window.motion = None;
            }
        }
        for closing in &mut self.closing {
            closing.motion.advance(tick);
        }
        self.animate_preview(tick);
        self.animate_desktop(tick);
        self.animate_slide_back(tick);
        self.animate_workspaces(tick);
        self.animate_overview(tick);
        self.animate_source_hud(tick.time);
        self.animate_switcher(tick);
        self.animate_assist(tick);
        self.animate_window_menu(tick);
        self.animate_zoom(tick.time);
        self.animate_flash(tick.time);
        self.animate_reveal(tick.time);
        self.closing.retain(|closing| !closing.motion.is_settled());
        if self.is_animating() {
            self.needs_redraw = true;
        }
    }

    fn is_animating(&self) -> bool {
        !self.closing.is_empty()
            || self.preview.is_some()
            || self.desktop.is_animating()
            || self.slide_back.is_some()
            || self.workspaces_animating()
            || self.overview.is_animating()
            || self.input_sources.is_animating()
            || self.switcher.is_animating()
            || self.assist.is_animating()
            || self.window_menus.is_animating()
            || self.zoom.is_animating()
            || self.is_flashing()
            || self.is_revealing()
            || self.windows.iter().any(|window| window.motion.is_some())
    }
}

impl Compositor {
    pub fn animate(&mut self, time: Duration) {
        self.state.animate(self.state.tick(time, 0));
    }

    pub fn animate_on(&mut self, output: &str, time: Duration) {
        let screen = self.state.screen_named(output).unwrap_or(0);
        self.state.animate(self.state.tick(time, screen));
    }

    pub fn set_reduce_motion(&mut self, reduce: bool) {
        self.state.reduce_motion = reduce;
    }

    pub fn is_animating(&self) -> bool {
        self.state.is_animating()
    }
}
