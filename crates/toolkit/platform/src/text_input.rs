use bande_graphics::{Event, Rect};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{self, ZwpTextInputV3};

use crate::window::{State, Window, WindowEvent};

type Preedit = (String, Option<(usize, usize)>);

#[derive(Default)]
pub(crate) struct TextInput {
    pub(crate) manager: Option<ZwpTextInputManagerV3>,
    text_input: Option<ZwpTextInputV3>,
    surface: Option<WlSurface>,
    entered: bool,
    enabled: bool,
    area: Option<Rect>,
    pending_commit: Option<String>,
    pending_preedit: Option<Preedit>,
}

impl State {
    pub(crate) fn join_text_input(&mut self, surface: &WlSurface, handle: &QueueHandle<Self>) {
        let (Some(manager), Some(seat)) = (&self.text_input.manager, &self.seat) else {
            return;
        };
        self.text_input.text_input = Some(manager.get_text_input(seat, handle, ()));
        self.text_input.surface = Some(surface.clone());
    }
}

fn preedit_cursor(text: &str, begin: i32, end: i32) -> Option<(usize, usize)> {
    let begin = usize::try_from(begin).ok()?;
    let end = usize::try_from(end).ok()?;
    let fits = |offset: usize| offset <= text.len() && text.is_char_boundary(offset);
    (fits(begin) && fits(end)).then_some((begin, end))
}

impl Window {
    pub(crate) fn reset_text_input(&mut self) {
        let input = &mut self.state.text_input;
        let Some(text_input) = &input.text_input else {
            return;
        };
        if !input.enabled {
            return;
        }
        text_input.disable();
        text_input.commit();
        input.enabled = false;
        input.area = None;
    }

    pub(crate) fn set_text_input_area(&mut self, area: Option<Rect>) {
        let offset = self.content_offset();
        let area = area.map(|rect| {
            Rect::new(
                rect.x + offset.x,
                rect.y + offset.y,
                rect.width,
                rect.height,
            )
        });
        let input = &mut self.state.text_input;
        let Some(text_input) = &input.text_input else {
            return;
        };
        if !input.entered || input.area == area {
            return;
        }
        match area {
            Some(rect) => {
                if !input.enabled {
                    text_input.enable();
                    input.enabled = true;
                }
                text_input.set_cursor_rectangle(
                    rect.x.round() as i32,
                    rect.y.round() as i32,
                    rect.width.ceil() as i32,
                    rect.height.ceil() as i32,
                );
                text_input.commit();
            }
            None => {
                text_input.disable();
                text_input.commit();
                input.enabled = false;
            }
        }
        input.area = area;
    }
}

impl Dispatch<ZwpTextInputV3, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let input = &mut state.text_input;
        match event {
            zwp_text_input_v3::Event::Enter { surface } => {
                input.entered = input.surface.as_ref() == Some(&surface);
                input.enabled = false;
                input.area = None;
            }
            zwp_text_input_v3::Event::Leave { .. } => {
                input.entered = false;
                input.enabled = false;
                input.area = None;
            }
            zwp_text_input_v3::Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                input.pending_preedit = text.map(|text| {
                    let cursor = preedit_cursor(&text, cursor_begin, cursor_end);
                    (text, cursor)
                });
            }
            zwp_text_input_v3::Event::CommitString { text } => input.pending_commit = text,
            zwp_text_input_v3::Event::Done { .. } => {
                if let Some(text) = input.pending_commit.take() {
                    state.events.push(WindowEvent::Input(Event::Commit(text)));
                }
                let (text, cursor) = input.pending_preedit.take().unwrap_or_default();
                state
                    .events
                    .push(WindowEvent::Input(Event::Preedit { text, cursor }));
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ZwpTextInputManagerV3);
