use smithay::desktop::layer_map_for_output;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle, Size};
use smithay::wayland::compositor::{TraversalAction, with_surface_tree_downward};
use smithay::wayland::fractional_scale::with_fractional_scale;
use unbound_spring::Spring;

use crate::motion::{Look, OPENING_SCALE};
use crate::screen::{OutputConfig, OutputSettings, Screen};
use crate::state::{Headless, Rect, State};

const MAIN_SCREEN_LAYERS: [&str; 6] = [
    "dock",
    "menubar",
    "desktop",
    "notifications",
    "greeter",
    "screenshot",
];

pub(crate) struct Departed {
    identity: String,
    spaces: Vec<u64>,
    landed: u64,
    windows: Vec<(WlSurface, Rect, bool)>,
}

fn center(rect: Rect) -> (f64, f64) {
    (
        f64::from(rect.location.x) + f64::from(rect.size.w) / 2.0,
        f64::from(rect.location.y) + f64::from(rect.size.h) / 2.0,
    )
}

pub(crate) fn carried(rect: Rect, from: Rect, to: Rect) -> Rect {
    let size: Size<i32, Logical> = (
        rect.size.w.min(to.size.w).max(1),
        rect.size.h.min(to.size.h).max(1),
    )
        .into();
    let (cx, cy) = center(rect);
    let rx = (cx - f64::from(from.location.x)) / f64::from(from.size.w.max(1));
    let ry = (cy - f64::from(from.location.y)) / f64::from(from.size.h.max(1));
    let x = f64::from(to.location.x) + rx * f64::from(to.size.w) - f64::from(size.w) / 2.0;
    let y = f64::from(to.location.y) + ry * f64::from(to.size.h) - f64::from(size.h) / 2.0;
    let x = (x.round() as i32).clamp(to.location.x, to.location.x + to.size.w - size.w);
    let y = (y.round() as i32).clamp(to.location.y, to.location.y + to.size.h - size.h);
    Rect {
        location: (x, y).into(),
        size,
    }
}

impl State {
    pub(crate) fn screen_for_layer(&self, namespace: &str) -> usize {
        if MAIN_SCREEN_LAYERS.contains(&namespace) {
            0
        } else {
            self.pointer_screen()
        }
    }

    pub(crate) fn screen_of_surface(&self, surface: &WlSurface) -> usize {
        let root = self.root_of(surface).unwrap_or_else(|| surface.clone());
        if let Some(index) = self.window_index(&root) {
            return self.screen_of(&self.windows[index]);
        }
        self.layer_screen(&root)
            .unwrap_or_else(|| self.pointer_screen())
    }

    pub(crate) fn sync_surface_outputs(&self) {
        for window in &self.windows {
            let home = self.screen_of(window);
            for (index, screen) in self.screens.iter().enumerate() {
                with_surface_tree_downward(
                    window.surface(),
                    (),
                    |_, _, ()| TraversalAction::DoChildren(()),
                    |surface, states, ()| {
                        if index == home {
                            screen.output.enter(surface);
                            with_fractional_scale(states, |fractional| {
                                fractional.set_preferred_scale(screen.scale());
                            });
                        } else {
                            screen.output.leave(surface);
                        }
                    },
                    |_, _, ()| true,
                );
            }
        }
        for screen in &self.screens {
            let map = layer_map_for_output(&screen.output);
            for layer in map.layers() {
                with_surface_tree_downward(
                    layer.wl_surface(),
                    (),
                    |_, _, ()| TraversalAction::DoChildren(()),
                    |_, states, ()| {
                        with_fractional_scale(states, |fractional| {
                            fractional.set_preferred_scale(screen.scale());
                        });
                    },
                    |_, _, ()| true,
                );
            }
        }
    }

    fn put_window(&mut self, index: usize, from: Rect, screen: usize, free: Option<Rect>) {
        let to = self.screen_rect(screen);
        let area = self.usable_area(screen);
        let window = &mut self.windows[index];
        window.motion = None;
        window.resize = None;
        if let Some(restore) = window.restore {
            window.restore = Some(carried(restore, from, area));
        }
        let placed = if window.has_state(xdg_toplevel::State::Fullscreen) {
            Some(to)
        } else if window.has_state(xdg_toplevel::State::Maximized) {
            Some(area)
        } else if let Some((tile, _)) = window.tile {
            let rect = tile.rect(area);
            window.tile = Some((tile, rect));
            Some(rect)
        } else {
            None
        };
        let Some(geometry) = window.geometry() else {
            return;
        };
        let target = placed
            .or(free.map(|free| carried(free, area, area)))
            .unwrap_or_else(|| carried(geometry, from, area));
        if target.size != geometry.size {
            window.request_size(Some(target.size));
            if let Some(serial) = window.configure() {
                window.pending_location = Some((serial, target.location));
            }
        } else {
            window.pending_location = None;
        }
        window.location = target.location;
        self.needs_redraw = true;
    }

    pub(crate) fn move_window_to_screen(
        &mut self,
        index: usize,
        screen: usize,
        workspace: Option<u64>,
    ) {
        let from_screen = self.screen_of(&self.windows[index]);
        if from_screen == screen {
            return;
        }
        let from = self.screen_rect(from_screen);
        let left = self.windows[index].workspace;
        let workspace = workspace.unwrap_or_else(|| self.screens[screen].workspaces.active_id());
        self.windows[index].workspace = workspace;
        self.put_window(index, from, screen, None);
        if self.present(&self.windows[index]) {
            let from = self.windows[index].geometry().map(|rect| {
                Look::hidden(Rectangle::new(rect.location.to_f64(), rect.size.to_f64()))
                    .scaled(OPENING_SCALE)
            });
            self.animate_window(index, Spring::STANDARD, from);
        }
        self.window_left_workspace(left);
        self.sync_surface_outputs();
        self.announce_workspaces();
    }

    pub(crate) fn dragged_onto(&self, window: &crate::Window, screen: usize) -> bool {
        let dragged = self
            .snap
            .as_ref()
            .is_some_and(|drag| drag.surface() == window.surface());
        let output = self.screen_rect(screen);
        dragged
            && window.geometry().is_some_and(|rect| {
                Rectangle::new(rect.location, rect.size)
                    .overlaps(Rectangle::new(output.location, output.size))
            })
    }

    pub(crate) fn screen_beside(&self, from: usize, direction: (i32, i32)) -> Option<usize> {
        let (cx, cy) = center(self.screen_rect(from));
        let (ux, uy) = (f64::from(direction.0), f64::from(direction.1));
        self.screens
            .iter()
            .enumerate()
            .filter(|(index, screen)| *index != from && screen.mirror.is_none())
            .filter_map(|(index, screen)| {
                let (x, y) = center(screen.rect());
                let (dx, dy) = (x - cx, y - cy);
                let along = dx * ux + dy * uy;
                let across = (dx * uy).abs() + (dy * ux).abs();
                (along > 0.0).then_some((index, along + 2.0 * across))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index)
    }

    pub(crate) fn move_to_screen_beside(&mut self, surface: &WlSurface, direction: (i32, i32)) {
        let Some(index) = self.window_index(surface) else {
            return;
        };
        let index = self.root_index(index);
        if self.on_full_screen_space(&self.windows[index]) {
            return;
        }
        let from = self.screen_of(&self.windows[index]);
        if let Some(screen) = self.screen_beside(from, direction) {
            self.move_window_to_screen(index, screen, None);
            self.focus(Some(surface.clone()));
        }
    }

    pub(crate) fn add_screen(&mut self, config: OutputConfig, settings: OutputSettings) {
        if self.screen_named(&config.name).is_some() {
            return;
        }
        let identity = config.identity();
        self.screens
            .push(Screen::new(&self.display_handle, config, settings));
        let screen = self.screens.len() - 1;
        self.bring_back(&identity, screen);
        self.after_screens_changed();
    }

    fn bring_back(&mut self, identity: &str, screen: usize) {
        let Some(at) = self
            .departed
            .iter()
            .position(|departed| departed.identity == identity)
        else {
            return;
        };
        let departed = self.departed.remove(at);
        let fresh = self.screens[screen].workspaces.active_id();
        let returning: Vec<(usize, Rect, Option<Rect>)> = departed
            .windows
            .iter()
            .filter_map(|(surface, rect, was_shown)| {
                let index = self.window_index(surface)?;
                let workspace = self.windows[index].workspace;
                let comes_back = if *was_shown {
                    workspace == departed.landed
                } else {
                    departed.spaces.contains(&workspace)
                };
                comes_back.then(|| {
                    let from = self.screen_rect(self.screen_of(&self.windows[index]));
                    (index, from, Some(*rect))
                })
            })
            .collect();
        let mut spaces = Vec::new();
        for holder in &mut self.screens {
            let held: Vec<u64> = holder
                .workspaces
                .ids()
                .into_iter()
                .filter(|id| departed.spaces.contains(id))
                .collect();
            if !held.is_empty() {
                spaces.extend(holder.workspaces.take(&held));
            }
        }
        self.screens[screen].workspaces.adopt(spaces);
        let origin = self.screen_rect(screen).location;
        for (index, from, rect) in returning {
            if !departed.spaces.contains(&self.windows[index].workspace) {
                self.windows[index].workspace = fresh;
            }
            let free = rect.map(|rect| Rect {
                location: origin + rect.location,
                size: rect.size,
            });
            self.put_window(index, from, screen, free);
        }
    }

    pub(crate) fn remove_screen(&mut self, screen: usize) {
        if self.screens.len() < 2 {
            return;
        }
        let from = self.screen_rect(screen);
        let identity = self.screens[screen].config.identity();
        let shown = self.screens[screen].workspaces.active_id();
        let spaces: Vec<u64> = self.screens[screen]
            .workspaces
            .ids()
            .into_iter()
            .filter(|id| *id != shown)
            .collect();
        let windows: Vec<(WlSurface, Rect, bool)> = self
            .windows
            .iter()
            .filter(|window| window.workspace == shown || spaces.contains(&window.workspace))
            .filter_map(|window| {
                let geometry = window.geometry()?;
                Some((
                    window.surface().clone(),
                    Rect {
                        location: geometry.location - from.location,
                        size: geometry.size,
                    },
                    window.workspace == shown,
                ))
            })
            .collect();
        let removed = self.screens.remove(screen);
        let taken = removed.workspaces.clone().take(&spaces);
        self.screens[0].workspaces.adopt(taken);
        let landed = self.screens[0].workspaces.active_id();
        for layer in layer_map_for_output(&removed.output).layers() {
            layer.layer_surface().send_close();
        }
        removed.retire(&self.display_handle);
        for index in 0..self.windows.len() {
            let workspace = self.windows[index].workspace;
            if workspace == shown {
                self.windows[index].workspace = landed;
            }
            if workspace == shown || spaces.contains(&workspace) {
                self.put_window(index, from, 0, None);
            }
        }
        self.departed
            .retain(|departed| departed.identity != identity);
        self.departed.push(Departed {
            identity,
            spaces,
            landed,
            windows,
        });
        self.after_screens_changed();
    }

    pub(crate) fn configure_screen(&mut self, screen: usize, settings: OutputSettings) {
        if self.screens[screen].settings == settings {
            return;
        }
        let before = self.screen_rect(screen);
        let pointer_here = self.screen_at(self.pointer_location) == Some(screen);
        self.screens[screen].apply(settings);
        let after = self.screen_rect(screen);
        if pointer_here {
            let relative = self.pointer_location - before.location.to_f64();
            self.pointer_location = after.location.to_f64()
                + Point::from((
                    relative.x * f64::from(after.size.w) / f64::from(before.size.w.max(1)),
                    relative.y * f64::from(after.size.h) / f64::from(before.size.h.max(1)),
                ));
        }
        if before != after {
            for index in 0..self.windows.len() {
                if self.screen_of(&self.windows[index]) != screen {
                    continue;
                }
                if before.size == after.size {
                    self.shift_window(index, after.location - before.location);
                } else {
                    self.put_window(index, before, screen, None);
                }
            }
        }
        self.after_screens_changed();
    }

    fn shift_window(&mut self, index: usize, by: Point<i32, Logical>) {
        let window = &mut self.windows[index];
        window.location += by;
        window.motion = None;
        if let Some(restore) = &mut window.restore {
            restore.location += by;
        }
        if let Some((_, rect)) = &mut window.tile {
            rect.location += by;
        }
        if let Some((_, at)) = &mut window.pending_location {
            *at += by;
        }
        if let Some(resize) = &mut window.resize {
            resize.initial.location += by;
        }
    }

    pub(crate) fn make_main(&mut self, screen: usize) {
        if screen == 0 || screen >= self.screens.len() {
            return;
        }
        let main = self.screens.remove(screen);
        self.screens.insert(0, main);
        self.after_screens_changed();
    }

    pub(crate) fn arrange_screens(&mut self) {
        let heads: Vec<OutputConfig> = self
            .screens
            .iter()
            .map(|screen| screen.config.clone())
            .collect();
        let main = self.main_screen().name();
        let arranged = crate::arrange::arrange(&heads, &main, &self.display_config);
        for display in &arranged {
            if let Some(screen) = self.screen_named(&display.name) {
                self.configure_screen(screen, display.settings);
            }
        }
        if let Some(main) = arranged
            .first()
            .and_then(|display| self.screen_named(&display.name))
        {
            self.make_main(main);
        }
        for display in &arranged {
            self.mirror_screen(&display.name, display.mirror.as_deref());
        }
    }

    fn mirror_screen(&mut self, name: &str, source: Option<&str>) {
        let Some(screen) = self.screen_named(name) else {
            return;
        };
        let source = source.and_then(|source| self.screen_named(source));
        let Some(source) = source else {
            if self.screens[screen].mirror.take().is_some() {
                self.after_screens_changed();
            }
            return;
        };
        let moved: Vec<usize> = (0..self.windows.len())
            .filter(|index| self.screen_of(&self.windows[*index]) == screen)
            .collect();
        let mirror = Some(self.screens[source].name());
        let remirrored = self.screens[screen].mirror != mirror;
        self.screens[screen].mirror = mirror;
        let mut changed = remirrored || !moved.is_empty();
        for index in moved {
            self.move_window_to_screen(index, source, None);
        }
        let shown = self.screen_rect(source);
        let current = self.screens[screen].settings;
        let frame = current.transform.transform_size(current.mode.size);
        let scale = (f64::from(frame.w) / f64::from(shown.size.w.max(1)))
            .min(f64::from(frame.h) / f64::from(shown.size.h.max(1)));
        let settings = OutputSettings {
            scale,
            position: shown.location,
            ..current
        };
        if settings != current {
            self.screens[screen].apply(settings);
            changed = true;
        }
        if changed {
            self.after_screens_changed();
        }
    }

    fn after_screens_changed(&mut self) {
        self.end_switcher();
        self.dismiss_assist();
        self.announce_outputs();
        self.pointer_location = self.nearest_on_screens(self.pointer_location);
        self.sync_surface_outputs();
        self.resize_lock_surfaces();
        self.reload_cursor();
        self.relayout_overview();
        self.needs_redraw = true;
        for screen in &mut self.screens {
            screen.redraw = true;
        }
        self.tidy_workspaces();
        self.refocus_pointer();
    }
}

impl Headless {
    pub fn outputs(&self) -> Vec<String> {
        self.state.screens.iter().map(Screen::name).collect()
    }

    pub fn output_rect(&self, output: &str) -> Option<Rect> {
        Some(self.state.screen_rect(self.state.screen_named(output)?))
    }

    pub fn output_settings(&self, output: &str) -> Option<OutputSettings> {
        Some(self.state.screens[self.state.screen_named(output)?].settings)
    }

    pub fn output_of(&self, window: &crate::Window) -> String {
        self.state.screens[self.state.screen_of(window)].name()
    }

    pub fn add_output(&mut self, config: OutputConfig) {
        let Some(mut settings) = config.default_settings() else {
            return;
        };
        let main = self.state.screen_rect(0);
        let right = self
            .state
            .screens
            .iter()
            .map(|screen| screen.rect())
            .map(|rect| rect.location.x + rect.size.w)
            .max()
            .unwrap_or(0);
        settings.position = (right, main.location.y).into();
        self.state.add_screen(config, settings);
        self.state.arrange_screens();
    }

    pub fn remove_output(&mut self, output: &str) {
        if let Some(screen) = self.state.screen_named(output) {
            self.state.remove_screen(screen);
            self.state.arrange_screens();
        }
    }

    pub fn set_display_config(&mut self, config: unbound_config::Config) {
        self.state.display_config = config;
        self.state.arrange_screens();
    }

    pub fn output_identity(&self, output: &str) -> Option<String> {
        let heads: Vec<OutputConfig> = self
            .state
            .screens
            .iter()
            .map(|screen| screen.config.clone())
            .collect();
        let index = self.state.screen_named(output)?;
        crate::arrange::identities(&heads).into_iter().nth(index)
    }

    pub fn configure_output(&mut self, output: &str, settings: OutputSettings) {
        if let Some(screen) = self.state.screen_named(output) {
            self.state.configure_screen(screen, settings);
        }
    }

    pub fn main_output(&self) -> String {
        self.state.main_screen().name()
    }
}
