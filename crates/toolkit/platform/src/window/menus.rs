use std::time::Instant;

use bande_graphics::{Appearance, Event, Point, Rect, TextSystem};
use bande_widgets::{FrameTime, Menu, MenuItem, MenuOutcome};

use super::{ShellSurface, Window, WindowError, WindowEvent};
use crate::L10N;
use crate::chrome::Control;
use crate::popup::{MenuPlacement, Menus, Opening, Parent};

fn menu_text(text: &mut Option<TextSystem>, appearance: Appearance) -> &mut TextSystem {
    let text = text.get_or_insert_with(TextSystem::new);
    text.set_size(appearance.text_size);
    text
}

impl Window {
    fn move_and_resize_items(&self) -> Vec<MenuItem> {
        let toplevel = self.state.toplevel;
        let place = |key: &str| MenuItem::new(&L10N.tr(key), key);
        let mut items = vec![
            place("place-left").shortcut("Ctrl+Super+Left"),
            place("place-right").shortcut("Ctrl+Super+Right"),
            MenuItem::separator(),
            place("place-top-left"),
            place("place-top-right"),
            place("place-bottom-left"),
            place("place-bottom-right"),
            MenuItem::separator(),
            place("place-fill").enabled(!toplevel.maximized),
        ];
        if self.can_place() && self.state.outputs.len() > 1 {
            items.push(place("place-next-display").shortcut("Alt+Shift+Super+Right"));
        }
        items.extend([
            MenuItem::separator(),
            place("place-restore").enabled(toplevel.tiled || toplevel.maximized),
        ]);
        items
    }

    pub(super) fn offers_tiling_menu(&self) -> bool {
        self.can_place() && !self.state.toplevel.fullscreen && self.menus.is_none()
    }

    pub(super) fn open_tiling_menu(&mut self) -> Result<(), WindowError> {
        let control = Control::Zoom.area(self.frame().title);
        let offset = self.content_offset();
        self.show_menu(
            Menu::new(self.move_and_resize_items()),
            MenuPlacement::Below(Rect::new(
                control.x - offset.x,
                control.y - offset.y,
                control.width,
                control.height,
            )),
        )?;
        self.window_menu = true;
        Ok(())
    }

    pub(super) fn open_window_menu(&mut self, at: Point) -> Result<(), WindowError> {
        let toplevel = self.state.toplevel;
        let mut items = vec![
            MenuItem::new(&L10N.tr("minimize"), "minimize")
                .enabled(!toplevel.fullscreen)
                .shortcut("Super+M"),
            MenuItem::new(
                &L10N.tr(if toplevel.maximized { "unzoom" } else { "zoom" }),
                "zoom",
            )
            .enabled(!toplevel.fullscreen),
            MenuItem::new(
                &L10N.tr(if toplevel.fullscreen {
                    "exit-full-screen"
                } else {
                    "enter-full-screen"
                }),
                "full-screen",
            )
            .shortcut("Ctrl+Super+F"),
            MenuItem::separator(),
        ];
        if self.can_place() {
            items.push(
                MenuItem::submenu(
                    &L10N.tr("move-and-resize"),
                    Menu::new(self.move_and_resize_items()),
                )
                .enabled(!toplevel.fullscreen),
            );
        }
        if self.can_keep_on_top() {
            items.push(
                MenuItem::new(&L10N.tr("keep-on-top"), "keep-on-top")
                    .checked(self.keep_on_top)
                    .enabled(!toplevel.fullscreen),
            );
        }
        items.extend([
            MenuItem::new(&L10N.tr("move"), "move").enabled(!toplevel.fullscreen),
            MenuItem::separator(),
            MenuItem::new(&L10N.tr("close"), "close").shortcut("Super+W"),
            MenuItem::new(&L10N.tr("quit-app"), "quit").shortcut("Super+Q"),
        ]);
        let offset = self.content_offset();
        self.show_menu(
            Menu::new(items),
            MenuPlacement::AtPoint(Point::new(at.x - offset.x, at.y - offset.y)),
        )?;
        self.window_menu = true;
        Ok(())
    }

    fn window_menu_action(&mut self, action: &str) -> Result<(), WindowError> {
        use bande_protocols::client::bande_app_v1::Placement;
        match action {
            "minimize" => self.minimize(),
            "zoom" => self.toggle_zoom(),
            "full-screen" => self.toggle_full_screen(),
            "move" => self.start_move(),
            "close" => {
                self.state.events.push(WindowEvent::CloseRequested);
                Ok(())
            }
            "quit" => {
                self.state.events.push(WindowEvent::QuitRequested);
                Ok(())
            }
            "place-left" => self.place(Placement::Left),
            "place-right" => self.place(Placement::Right),
            "place-top-left" => self.place(Placement::TopLeft),
            "place-top-right" => self.place(Placement::TopRight),
            "place-bottom-left" => self.place(Placement::BottomLeft),
            "place-bottom-right" => self.place(Placement::BottomRight),
            "place-fill" if !self.state.toplevel.maximized => self.toggle_zoom(),
            "place-next-display" => self.place(Placement::NextDisplay),
            "place-restore" => self.place(Placement::Restore),
            "keep-on-top" => {
                let keep = !self.keep_on_top;
                self.set_keep_on_top(keep)?;
                self.keep_on_top = keep;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub fn show_menu(&mut self, menu: Menu, placement: MenuPlacement) -> Result<(), WindowError> {
        self.close_menu();
        let offset = self.content_offset();
        let placement = match placement {
            MenuPlacement::AtPoint(point) => {
                MenuPlacement::AtPoint(Point::new(point.x + offset.x, point.y + offset.y))
            }
            MenuPlacement::Below(rect) => MenuPlacement::Below(Rect::new(
                rect.x + offset.x,
                rect.y + offset.y,
                rect.width,
                rect.height,
            )),
        };
        let parent = match &self.role {
            ShellSurface::Toplevel(xdg_surface, _) => Parent::Toplevel(xdg_surface.clone()),
            ShellSurface::Layer(layer) => Parent::Layer(layer.clone()),
            ShellSurface::Lock(_) => {
                return Err(WindowError::Missing("a surface that takes popups"));
            }
        };
        let frame = self.frame_time();
        let opacity = self.fading.take().map_or(0.0, |fading| {
            let opacity = fading.opacity(frame.on_screen());
            fading.destroy();
            opacity
        });
        let handle = self.queue.handle();
        let appearance = self.appearance();
        let opening = Opening {
            state: &mut self.state,
            handle: &handle,
            text: menu_text(&mut self.text, appearance),
            palette: appearance.palette(),
        };
        let menus = Menus::open(opening, menu, parent, placement, opacity)?;
        self.menus = Some(menus);
        self.flush()?;
        while self.menus.as_ref().is_some_and(Menus::waiting) {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        self.draw_menus(self.frame_time())
    }

    pub fn is_menu_open(&self) -> bool {
        self.menus.is_some()
    }

    pub fn close_menu(&mut self) {
        self.window_menu = false;
        if let Some(menus) = self.menus.take() {
            if let Some(fading) = self.fading.take() {
                fading.destroy();
            }
            let handle = self.queue.handle();
            let now = self.frame_time().on_screen();
            self.fading = Some(menus.close(&mut self.state, &handle, now));
            self.state.popup_input.clear();
            self.accessibility.request_tree();
            let _ = self.flush();
        }
    }

    fn draw_fading(&mut self, frame: FrameTime) {
        let handle = self.queue.handle();
        let Some(fading) = &mut self.fading else {
            return;
        };
        match fading.draw(&self.state, &handle, frame) {
            Ok(true) => {}
            Ok(false) | Err(_) => {
                if let Some(fading) = self.fading.take() {
                    fading.destroy();
                }
            }
        }
        let _ = self.flush();
    }

    pub(super) fn draw_menus(&mut self, now: FrameTime) -> Result<(), WindowError> {
        let appearance = self.appearance.borrow().current();
        let scale = self.scale();
        let handle = self.queue.handle();
        if let Some(menus) = &mut self.menus {
            menus.draw(
                &mut self.state,
                &handle,
                &mut self.cpu,
                appearance,
                scale,
                now,
            )?;
        }
        self.accessibility.request_tree();
        self.flush()
    }

    pub(super) fn serve_menus(&mut self, frame: FrameTime) {
        let now = frame.on_screen();
        let changed = std::mem::take(&mut self.state.popups_changed);
        if changed && self.fading.is_some() {
            self.draw_fading(frame);
        }
        if self.menus.is_none() {
            self.state.popup_input.clear();
            return;
        }
        let mut outcomes = Vec::new();
        if self.menus.as_ref().is_some_and(Menus::dismissed) {
            outcomes.push((MenuOutcome::Dismiss, false));
        }
        let input = std::mem::take(&mut self.state.popup_input);
        if let Some(menus) = &mut self.menus {
            for (level, event) in input {
                let keyboard = matches!(event, Event::Key { .. });
                if keyboard && menus.tracker().is_flashing() {
                    self.state.events.push(WindowEvent::Input(event));
                    continue;
                }
                outcomes.push((menus.handle(level, &event, now), keyboard));
            }
            outcomes.push((menus.tick(Instant::now().max(now)), false));
        }
        let mut redraw = changed
            || self
                .menus
                .as_ref()
                .is_some_and(|menus| menus.tracker().is_flashing());
        for (outcome, keyboard) in outcomes {
            redraw |= self.apply_menu_outcome(outcome, keyboard);
        }
        if redraw
            && self.menus.is_some()
            && let Err(error) = self.draw_menus(frame)
        {
            tracing::warn!(%error, "cannot draw the menus");
        }
    }

    pub(super) fn apply_menu_outcome(&mut self, outcome: MenuOutcome, keyboard: bool) -> bool {
        match outcome {
            MenuOutcome::Nothing => false,
            MenuOutcome::Redraw => true,
            MenuOutcome::Activate(action) if self.window_menu => {
                if let Err(error) = self.window_menu_action(&action) {
                    tracing::warn!(%error, %action, "cannot perform the window menu action");
                }
                true
            }
            MenuOutcome::Activate(action) => {
                self.state.events.push(WindowEvent::MenuAction(action));
                true
            }
            MenuOutcome::Dismiss => {
                let activated = self
                    .menus
                    .as_ref()
                    .is_some_and(|menus| menus.tracker().is_flashing());
                let window_menu = self.window_menu;
                self.close_menu();
                if !activated && !window_menu {
                    self.state.events.push(WindowEvent::MenuClosed);
                }
                false
            }
            MenuOutcome::CloseFrom(level) => {
                if let Some(menus) = &mut self.menus {
                    menus.close_from(&mut self.state, level);
                }
                true
            }
            MenuOutcome::Search(menu) => {
                let handle = self.queue.handle();
                let appearance = self.appearance();
                if let Some(menus) = &mut self.menus {
                    let opening = Opening {
                        state: &mut self.state,
                        handle: &handle,
                        text: menu_text(&mut self.text, appearance),
                        palette: appearance.palette(),
                    };
                    if let Err(error) = menus.search(opening, menu) {
                        tracing::warn!(%error, "cannot resize the searched menu");
                    }
                }
                true
            }
            MenuOutcome::OpenSubmenu { level, index } => {
                let handle = self.queue.handle();
                let appearance = self.appearance();
                if let Some(menus) = &mut self.menus {
                    let opening = Opening {
                        state: &mut self.state,
                        handle: &handle,
                        text: menu_text(&mut self.text, appearance),
                        palette: appearance.palette(),
                    };
                    if let Err(error) = menus.open_submenu(opening, level, index, keyboard) {
                        tracing::warn!(%error, "cannot open the submenu");
                    }
                }
                true
            }
        }
    }
}
