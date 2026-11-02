use accesskit::ActionRequest;
use bande_graphics::{Cursor, Event, Modifiers, Point, Pointer, PointerButton, ResizeEdge, Scroll};

use super::{Window, WindowError, WindowEvent};
use crate::chrome::{Control, Frame, Zone, control_for_node};
use crate::clipboard::DragEvent;
use crate::seat::CLICK_SLOP;

fn resize_cursor(edge: ResizeEdge) -> Cursor {
    match edge {
        ResizeEdge::Top => Cursor::ResizeTop,
        ResizeEdge::Bottom => Cursor::ResizeBottom,
        ResizeEdge::Left => Cursor::ResizeLeft,
        ResizeEdge::Right => Cursor::ResizeRight,
        ResizeEdge::TopLeft => Cursor::ResizeTopLeft,
        ResizeEdge::TopRight => Cursor::ResizeTopRight,
        ResizeEdge::BottomLeft => Cursor::ResizeBottomLeft,
        ResizeEdge::BottomRight => Cursor::ResizeBottomRight,
    }
}

fn relative_to(origin: Point, point: Point) -> Point {
    Point::new(point.x - origin.x, point.y - origin.y)
}

fn into_content(frame: &Frame, pointer: Pointer) -> Pointer {
    Pointer {
        position: relative_to(frame.content_origin(), pointer.position),
        ..pointer
    }
}

impl Window {
    pub(super) fn through_chrome(&mut self, event: WindowEvent, passed: &mut Vec<WindowEvent>) {
        if self.chrome.is_none() {
            passed.push(event);
            return;
        }
        match event {
            WindowEvent::Input(input) => self.input_through_chrome(input, passed),
            WindowEvent::Drag(drag) => passed.push(WindowEvent::Drag(self.drag_into_content(drag))),
            WindowEvent::ToplevelChanged(_) => {
                self.needs_redraw = true;
                passed.push(event);
            }
            other => passed.push(other),
        }
    }

    fn drag_into_content(&self, drag: DragEvent) -> DragEvent {
        let origin = self.frame().content_origin();
        match drag {
            DragEvent::Entered {
                position,
                uris,
                text,
            } => DragEvent::Entered {
                position: relative_to(origin, position),
                uris,
                text,
            },
            DragEvent::Moved(position) => DragEvent::Moved(relative_to(origin, position)),
            DragEvent::Dropped {
                position,
                transfer,
                action,
            } => DragEvent::Dropped {
                position: relative_to(origin, position),
                transfer,
                action,
            },
            other => other,
        }
    }

    fn input_through_chrome(&mut self, input: Event, passed: &mut Vec<WindowEvent>) {
        let frame = self.frame();
        match input {
            Event::PointerEnter(pointer) | Event::PointerMove(pointer) => {
                self.pointer_moved_over_chrome(&frame, pointer, passed);
            }
            Event::PointerDown(pointer) => self.pointer_pressed_on_chrome(&frame, pointer, passed),
            Event::PointerUp(pointer) => self.pointer_released_on_chrome(&frame, pointer, passed),
            Event::ModifiersChanged(modifiers) => {
                self.modifiers_changed_on_chrome(modifiers);
                passed.push(WindowEvent::Input(input));
            }
            Event::PointerLeave => self.pointer_left_chrome(&frame, passed),
            Event::Scroll(scroll) => {
                if let Some(chrome) = &self.chrome
                    && chrome.zone_at(&frame, scroll.position) == Zone::Content
                {
                    passed.push(WindowEvent::Input(Event::Scroll(Scroll {
                        position: relative_to(frame.content_origin(), scroll.position),
                        ..scroll
                    })));
                }
            }
            other => passed.push(WindowEvent::Input(other)),
        }
    }

    fn pointer_moved_over_chrome(
        &mut self,
        frame: &Frame,
        pointer: Pointer,
        passed: &mut Vec<WindowEvent>,
    ) {
        let now = self.frame_time().on_screen();
        let reduce_motion = self.appearance.borrow().current().reduce_motion;
        let Some(chrome) = self.chrome.as_mut() else {
            return;
        };
        if let Some(start) = chrome.tab_press
            && (pointer.position.x - start.x).hypot(pointer.position.y - start.y) > CLICK_SLOP
        {
            chrome.tab_press = None;
            chrome.content_buttons = 0;
            if std::mem::take(&mut chrome.pointer_in_content) {
                passed.push(WindowEvent::Input(Event::PointerLeave));
            }
            if self.start_move().is_err() {
                self.needs_redraw = true;
            }
            return;
        }
        let zone = chrome.zone_at(frame, pointer.position);
        if let Some(control) = chrome.pressed {
            let inside = zone == Zone::Control(control);
            if inside != chrome.pressed_inside {
                chrome.pressed_inside = inside;
                chrome.zoom_rest = None;
                self.needs_redraw = true;
            }
            return;
        }
        if chrome.hover(frame, Some(pointer.position), now, reduce_motion) {
            self.needs_redraw = true;
        }
        let content = zone == Zone::Content || chrome.content_buttons > 0;
        if content {
            let entering = !chrome.pointer_in_content;
            chrome.pointer_in_content = true;
            let pointer = into_content(frame, pointer);
            passed.push(WindowEvent::Input(if entering {
                Event::PointerEnter(pointer)
            } else {
                Event::PointerMove(pointer)
            }));
        } else if chrome.pointer_in_content {
            chrome.pointer_in_content = false;
            passed.push(WindowEvent::Input(Event::PointerLeave));
        }
        let cursor = match zone {
            _ if content => self.app_cursor,
            Zone::Resize(edge) => resize_cursor(edge),
            _ => Cursor::Default,
        };
        self.state.input.set_cursor(cursor);
    }

    fn pointer_pressed_on_chrome(
        &mut self,
        frame: &Frame,
        pointer: Pointer,
        passed: &mut Vec<WindowEvent>,
    ) {
        let now = self.frame_time().on_screen();
        let Some(chrome) = self.chrome.as_mut() else {
            return;
        };
        let zone = chrome.zone_at(frame, pointer.position);
        if zone == Zone::Content || chrome.content_buttons > 0 {
            if chrome.content_buttons == 0
                && pointer.button == PointerButton::Primary
                && chrome.tab_at(frame, pointer.position)
            {
                chrome.tab_press = Some(pointer.position);
            }
            chrome.content_buttons += 1;
            passed.push(WindowEvent::Input(Event::PointerDown(into_content(
                frame, pointer,
            ))));
            return;
        }
        if pointer.button == PointerButton::Secondary
            && matches!(zone, Zone::TitleBar | Zone::Control(_))
        {
            if self.open_window_menu(pointer.position).is_err() {
                self.needs_redraw = true;
            }
            return;
        }
        if pointer.button != PointerButton::Primary {
            return;
        }
        let result = match zone {
            Zone::Control(control) => {
                chrome.pressed = Some(control);
                chrome.pressed_inside = true;
                chrome.zoom_rest = (control == Control::Zoom).then_some(now);
                self.needs_redraw = true;
                match control {
                    Control::Menu => self.show_menus(),
                    _ => Ok(()),
                }
            }
            Zone::TitleBar if pointer.count.is_multiple_of(2) => self.toggle_zoom(),
            Zone::TitleBar => self.start_move(),
            Zone::Resize(edge) => self.start_resize(edge),
            Zone::Content | Zone::Outside => Ok(()),
        };
        if result.is_err() {
            self.needs_redraw = true;
        }
    }

    fn pointer_released_on_chrome(
        &mut self,
        frame: &Frame,
        pointer: Pointer,
        passed: &mut Vec<WindowEvent>,
    ) {
        let Some(chrome) = self.chrome.as_mut() else {
            return;
        };
        chrome.tab_press = None;
        chrome.zoom_rest = None;
        if chrome.content_buttons > 0 {
            chrome.content_buttons -= 1;
            passed.push(WindowEvent::Input(Event::PointerUp(into_content(
                frame, pointer,
            ))));
            return;
        }
        let Some(control) = chrome.pressed.take() else {
            return;
        };
        self.needs_redraw = true;
        if chrome.zone_at(frame, pointer.position) != Zone::Control(control) {
            return;
        }
        let result = match control {
            Control::Close => {
                passed.push(WindowEvent::CloseRequested);
                Ok(())
            }
            Control::Minimize => self.minimize(),
            Control::Zoom if pointer.modifiers.alt => self.toggle_full_screen(),
            Control::Zoom => self.toggle_zoom(),
            Control::Menu => Ok(()),
        };
        if result.is_err() {
            self.needs_redraw = true;
        }
    }

    fn modifiers_changed_on_chrome(&mut self, modifiers: Modifiers) {
        if let Some(chrome) = &mut self.chrome
            && chrome.alternate != modifiers.alt
        {
            chrome.alternate = modifiers.alt;
            self.needs_redraw = true;
        }
    }

    fn pointer_left_chrome(&mut self, frame: &Frame, passed: &mut Vec<WindowEvent>) {
        let now = self.frame_time().on_screen();
        let reduce_motion = self.appearance.borrow().current().reduce_motion;
        let Some(chrome) = self.chrome.as_mut() else {
            return;
        };
        chrome.content_buttons = 0;
        chrome.tab_press = None;
        chrome.zoom_rest = None;
        if chrome.hover(frame, None, now, reduce_motion) | chrome.pressed.take().is_some() {
            self.needs_redraw = true;
        }
        if std::mem::take(&mut chrome.pointer_in_content) {
            passed.push(WindowEvent::Input(Event::PointerLeave));
        }
    }

    pub(super) fn perform_chrome(&mut self, request: &ActionRequest) -> bool {
        let Some(control) = control_for_node(request.target_node) else {
            return false;
        };
        if self.chrome.is_none() || request.action != accesskit::Action::Click {
            return false;
        }
        let result = match control {
            Control::Close => {
                self.request_close();
                Ok(())
            }
            Control::Minimize => self.minimize(),
            Control::Zoom => self.toggle_zoom(),
            Control::Menu => self.show_menus(),
        };
        result.is_ok()
    }

    pub(crate) fn announce_menu_button(&mut self) {
        let frame = self.frame();
        let Some(chrome) = &mut self.chrome else {
            return;
        };
        let button = frame.menu_button();
        if chrome.announced_menu == Some(button) {
            return;
        }
        chrome.announced_menu = Some(button);
        let (Ok(app), Ok(toplevel)) = (self.menu_app(), self.toplevel()) else {
            return;
        };
        let rect = button.unwrap_or_default();
        app.set_menu_button(
            toplevel,
            rect.x.round() as i32,
            rect.y.round() as i32,
            rect.width.round() as i32,
            rect.height.round() as i32,
        );
    }

    pub(crate) fn show_menus(&mut self) -> Result<(), WindowError> {
        let (seat, serial) = self.grab()?;
        self.menu_app()?.show_menus(self.toplevel()?, seat, serial);
        self.flush()
    }
}
