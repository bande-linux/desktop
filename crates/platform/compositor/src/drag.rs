use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::element::surface::render_elements_from_surface_tree;
use smithay::desktop::PopupUngrabStrategy;
use smithay::input::Seat;
use smithay::input::pointer::{CursorIcon, MotionEvent};
use smithay::reexports::wayland_server::protocol::wl_data_device_manager::DndAction;
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Clock, IsAlive, Logical, Monotonic, Point, Rectangle, SERIAL_COUNTER};
use smithay::wayland::compositor::{SurfaceAttributes, with_states};
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
    default_action_chooser,
};
use unbound_render::VulkanRenderer;
use unbound_spring::Spring;

use crate::motion::{Look, Motion, Tick};
use crate::scene::{Area, Piece, SlideBack, area_of, pieces_of};
use crate::state::{OutputElement, State};

pub(crate) struct Drag {
    source: Option<WlDataSource>,
    icon: Option<WlSurface>,
    over: Option<WlSurface>,
    action: DndAction,
    offset: Point<i32, Logical>,
    origin: Point<f64, Logical>,
    drawn: Option<(Vec<Piece>, Area)>,
}

fn take_offset(surface: &WlSurface) -> Point<i32, Logical> {
    with_states(surface, |states| {
        states
            .cached_state
            .get::<SurfaceAttributes>()
            .current()
            .buffer_delta
            .take()
            .unwrap_or_default()
    })
}

impl State {
    pub(crate) fn drag_cursor(&self) -> Option<CursorIcon> {
        let action = self.drag.as_ref()?.action;
        Some(if action.contains(DndAction::Copy) {
            CursorIcon::Copy
        } else if action.is_empty() {
            CursorIcon::NoDrop
        } else {
            CursorIcon::Move
        })
    }

    fn show_drag_action(&mut self, action: DndAction) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if drag.action == action {
            return;
        }
        drag.action = action;
        if let Some(cursor) = self.drag_cursor() {
            self.load_named_cursor(cursor);
        }
        self.needs_redraw = true;
    }

    pub(crate) fn drag_over(&mut self, surface: Option<&WlSurface>) {
        if self
            .drag
            .as_ref()
            .is_none_or(|drag| drag.over.as_ref() == surface)
        {
            return;
        }
        if let Some(drag) = self.drag.as_mut() {
            drag.over = surface.cloned();
        }
        self.show_drag_action(DndAction::empty());
    }

    pub(crate) fn renegotiate_drag(&mut self) {
        let Some(pointer) = self.seat.get_pointer().filter(|_| self.drag.is_some()) else {
            return;
        };
        let under = self.surface_under(self.pointer_location);
        self.drag_over(under.as_ref().map(|(surface, _)| surface));
        pointer.motion(
            self,
            under,
            &MotionEvent {
                location: self.pointer_location,
                serial: SERIAL_COUNTER.next_serial(),
                time: Clock::<Monotonic>::new().now().as_millis(),
            },
        );
        pointer.frame(self);
    }

    pub(crate) fn end_orphaned_drag(&mut self) {
        let orphaned = self.drag.as_ref().is_some_and(|drag| {
            drag.source.as_ref().is_some_and(|source| !source.alive())
                || drag.over.as_ref().is_some_and(|surface| !surface.alive())
        });
        if orphaned {
            self.renegotiate_drag();
        }
    }

    pub(crate) fn commit_drag_icon(&mut self, surface: &WlSurface) -> bool {
        let Some(drag) = self
            .drag
            .as_mut()
            .filter(|drag| drag.icon.as_ref() == Some(surface))
        else {
            return false;
        };
        drag.offset += take_offset(surface);
        self.needs_redraw = true;
        true
    }

    pub(crate) fn drag_icon_elements(
        &self,
        renderer: &mut VulkanRenderer,
        scale: f64,
    ) -> (Vec<OutputElement>, Option<(Vec<Piece>, Area)>) {
        let slid = self
            .slide_back
            .iter()
            .flat_map(|slide| slide.render_elements(scale));
        let Some((drag, icon)) = self
            .drag
            .as_ref()
            .and_then(|drag| Some((drag, drag.icon.as_ref()?)))
        else {
            return (slid.collect(), None);
        };
        let at = (self.pointer_location + drag.offset.to_f64())
            .to_physical(scale)
            .to_i32_round();
        let surfaces =
            render_elements_from_surface_tree(renderer, icon, at, scale, 1.0, Kind::Unspecified);
        let pieces = pieces_of(&surfaces, scale);
        let drawn = area_of(&pieces).map(|area| (pieces, area));
        (
            surfaces
                .into_iter()
                .map(OutputElement::Surface)
                .chain(slid)
                .collect(),
            drawn,
        )
    }

    pub(crate) fn remember_drag_icon(&mut self, drawn: Option<(Vec<Piece>, Area)>) {
        if let Some(drag) = self.drag.as_mut()
            && drawn.is_some()
        {
            drag.drawn = drawn;
        }
    }

    pub(crate) fn animate_slide_back(&mut self, tick: Tick) {
        let Some(slide) = self.slide_back.as_mut() else {
            return;
        };
        slide.motion.advance(tick);
        if slide.motion.is_settled() {
            self.slide_back = None;
        }
        self.needs_redraw = true;
    }

    fn slide_back_to_origin(&mut self, drag: Drag) {
        let Some((pieces, area)) = drag.drawn else {
            return;
        };
        let scale = self.screens[self.pointer_screen()].scale();
        let shown: Rectangle<f64, Logical> = area.to_logical(scale);

        let home = Rectangle::new(
            shown.loc + (drag.origin - self.pointer_location),
            shown.size,
        );
        let motion = if self.reduce_motion {
            Motion::new(Spring::FADE, Look::shown(shown), Look::hidden(shown))
        } else {
            Motion::new(Spring::STANDARD, Look::shown(shown), Look::shown(home))
        };
        self.slide_back = Some(SlideBack::new(pieces, area, motion));
    }
}

impl crate::state::Compositor {
    pub fn is_dragging(&self) -> bool {
        self.state.drag.is_some()
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.globals.data_device_state
    }

    fn action_choice(&mut self, available: DndAction, preferred: DndAction) -> DndAction {
        let modifiers = self
            .seat
            .get_keyboard()
            .map(|keyboard| keyboard.modifier_state());
        let wanted = match modifiers {
            Some(modifiers) if modifiers.alt => Some(DndAction::Copy),
            Some(modifiers) if modifiers.logo => Some(DndAction::Move),
            _ => None,
        };
        let chosen = wanted
            .filter(|action| available.contains(*action))
            .unwrap_or_else(|| default_action_chooser(available, preferred));
        self.show_drag_action(chosen);
        chosen
    }
}

impl ClientDndGrabHandler for State {
    fn started(
        &mut self,
        source: Option<WlDataSource>,
        icon: Option<WlSurface>,
        _seat: Seat<Self>,
    ) {
        let offset = icon.as_ref().map(take_offset).unwrap_or_default();
        if let Some(mut menu) = self.popup_grab.take() {
            menu.ungrab(PopupUngrabStrategy::All);
        }
        self.slide_back = None;
        self.drag = Some(Drag {
            source,
            icon,
            over: None,
            action: DndAction::empty(),
            offset,
            origin: self.pointer_location,
            drawn: None,
        });
        self.load_named_cursor(CursorIcon::NoDrop);
        self.needs_redraw = true;
    }

    fn dropped(&mut self, _target: Option<WlSurface>, validated: bool, _seat: Seat<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if !validated {
            self.slide_back_to_origin(drag);
        }
        self.needs_redraw = true;
    }
}

impl ServerDndGrabHandler for State {}
