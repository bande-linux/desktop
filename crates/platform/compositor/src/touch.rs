use std::collections::HashMap;

use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, Device, Event, InputBackend, TabletToolButtonEvent,
    TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState, TouchEvent,
    TouchSlot,
};
use smithay::input::touch::{DownEvent, MotionEvent, UpEvent};
use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::zwp_tablet_manager_v2::ZwpTabletManagerV2;
use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::zwp_tablet_seat_v2::ZwpTabletSeatV2;
use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::zwp_tablet_tool_v2::ZwpTabletToolV2;
use smithay::reexports::wayland_protocols::wp::tablet::zv2::server::zwp_tablet_v2::ZwpTabletV2;
use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Point, SERIAL_COUNTER, Serial};
use smithay::wayland::tablet_manager::{
    TabletDescriptor, TabletManagerState, TabletSeatTrait, TabletSeatUserData, TabletToolUserData,
    TabletUserData,
};

use crate::frame::Target;
use crate::input::BTN_LEFT;
use crate::state::{Compositor, State};

const EDGE_ZONE: f64 = 12.0;
const EDGE_SWIPE: f64 = 80.0;

#[derive(Debug, Clone)]
enum Contact {
    Client(Serial, WlSurface),
    Pointer,
    BottomEdge(Point<f64, Logical>),
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PenPress {
    Client,
    Pointer,
}

#[derive(Default)]
pub(crate) struct Touch {
    contacts: HashMap<TouchSlot, Contact>,
    location: Point<f64, Logical>,
    pen_focus: Option<(WlSurface, Point<f64, Logical>)>,
    pen_press: Option<PenPress>,
    tablet_clients: Vec<ClientId>,
    outputs: HashMap<String, Option<String>>,
}

fn wanted_output(device: &impl Device) -> Option<String> {
    let path = device.syspath()?;
    let udev = smithay::reexports::udev::Device::from_syspath(&path)
        .inspect_err(|error| tracing::debug!("cannot read {}: {error}", path.display()))
        .ok()?;
    udev.property_value("WL_OUTPUT")
        .and_then(|output| output.to_str())
        .map(str::to_owned)
}

impl Touch {
    fn pointer_held(&self) -> bool {
        self.contacts
            .values()
            .any(|contact| matches!(contact, Contact::Pointer))
    }

    pub(crate) fn forget_device(&mut self, device: &impl Device) {
        self.outputs.remove(&device.id());
    }
}

impl State {
    fn compositor_takes_touch(&self) -> bool {
        self.window_menus.is_open()
            || self.overview.is_open()
            || self.switcher.is_shown()
            || self.assist.is_open()
            || self.force_quit.is_open()
            || self.popup_grab.is_some()
    }

    fn client_under(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        if self.compositor_takes_touch() {
            return None;
        }
        let (surface, origin) = self
            .pointer_target(location)
            .as_ref()
            .and_then(Target::surface)?;
        let blocked = self
            .toplevel_of(&surface)
            .is_some_and(|window| self.modal_blocker(&window).is_some());
        (!blocked).then_some((surface, origin))
    }

    fn at_bottom_edge(&self, location: Point<f64, Logical>) -> bool {
        self.screen_at(location).is_some_and(|screen| {
            let rect = self.screen_rect(screen).to_f64();
            location.y >= rect.loc.y + rect.size.h - EDGE_ZONE
        }) && !self.compositor_takes_touch()
            && !self.is_locked()
            && !self.login_window
    }

    fn touch_serial_of(&self, serial: Serial, surface: &WlSurface) -> Option<TouchSlot> {
        self.touch
            .contacts
            .iter()
            .find_map(|(slot, contact)| match contact {
                Contact::Client(down, touched)
                    if *down == serial && touched.client() == surface.client() =>
                {
                    Some(*slot)
                }
                _ => None,
            })
    }

    fn input_screen(&mut self, device: &impl Device) -> usize {
        let wanted = self
            .touch
            .outputs
            .entry(device.id())
            .or_insert_with(|| wanted_output(device))
            .clone();
        wanted
            .as_deref()
            .and_then(|name| self.screen_named(name))
            .or_else(|| {
                self.screens
                    .iter()
                    .position(|screen| bande_edid::is_internal_connector(&screen.name()))
            })
            .unwrap_or(0)
    }

    fn pen_reaches(&self, surface: &WlSurface) -> bool {
        surface
            .client()
            .is_some_and(|client| self.touch.tablet_clients.contains(&client.id()))
    }
}

fn mapped<B: InputBackend>(
    state: &mut State,
    event: &impl AbsolutePositionEvent<B>,
) -> Point<f64, Logical> {
    let screen = state.input_screen(&event.device());
    let rect = state.screen_rect(screen);
    let transform = state.screens[screen].output.current_transform();
    let native = transform.invert().transform_size(rect.size);
    transform.transform_point_in(event.position_transformed(native), &native.to_f64())
        + rect.loc.to_f64()
}

impl Compositor {
    fn touch_down<B: InputBackend>(
        &mut self,
        event: &impl TouchEvent<B>,
        location: Point<f64, Logical>,
    ) {
        let slot = event.slot();
        let time = event.time_msec();
        let state = &mut self.state;
        state.touch.location = location;
        let contact = if state.at_bottom_edge(location) {
            Contact::BottomEdge(location)
        } else if let Some((surface, origin)) = state.client_under(location) {
            let serial = SERIAL_COUNTER.next_serial();
            if let Some(focusable) = state
                .toplevel_of(&surface)
                .or_else(|| state.focusable_layer_of(&surface))
            {
                state.focus(Some(focusable));
            }
            if let Some(touch) = state.seat.get_touch() {
                touch.down(
                    state,
                    Some((surface.clone(), origin)),
                    &DownEvent {
                        slot,
                        location,
                        serial,
                        time,
                    },
                );
            }
            Contact::Client(serial, surface)
        } else if state.touch.pointer_held() {
            Contact::Ignored
        } else {
            self.move_pointer_to(location, time);
            self.press_button(BTN_LEFT, true, time);
            Contact::Pointer
        };
        self.state.touch.contacts.insert(slot, contact);
    }

    fn touch_motion<B: InputBackend>(
        &mut self,
        event: &impl TouchEvent<B>,
        location: Point<f64, Logical>,
    ) {
        let slot = event.slot();
        let time = event.time_msec();
        match self.state.touch.contacts.get(&slot).cloned() {
            Some(Contact::Client(..)) => {
                let state = &mut self.state;
                let under = state.client_under(location);
                if let Some(touch) = state.seat.get_touch() {
                    touch.motion(
                        state,
                        under,
                        &MotionEvent {
                            slot,
                            location,
                            time,
                        },
                    );
                }
            }
            Some(Contact::Pointer) => self.move_pointer_to(location, time),
            Some(Contact::BottomEdge(start)) if start.y - location.y >= EDGE_SWIPE => {
                self.state.toggle_overview(None);
                self.state.touch.contacts.insert(slot, Contact::Ignored);
            }
            _ => {}
        }
    }

    fn lift(&mut self, slot: TouchSlot, time: u32) {
        match self.state.touch.contacts.remove(&slot) {
            Some(Contact::Client(..)) => {
                let state = &mut self.state;
                if let Some(touch) = state.seat.get_touch() {
                    touch.up(
                        state,
                        &UpEvent {
                            slot,
                            serial: SERIAL_COUNTER.next_serial(),
                            time,
                        },
                    );
                }
            }
            Some(Contact::Pointer) => self.press_button(BTN_LEFT, false, time),
            _ => {}
        }
    }

    pub(crate) fn handle_touch<B: InputBackend>(
        &mut self,
        event: smithay::backend::input::InputEvent<B>,
    ) {
        use smithay::backend::input::InputEvent;
        match event {
            InputEvent::TouchDown { event } => {
                let location = mapped(&mut self.state, &event);
                self.touch_down(&event, location);
            }
            InputEvent::TouchMotion { event } => {
                let location = mapped(&mut self.state, &event);
                self.touch_motion(&event, location);
            }
            InputEvent::TouchUp { event } => self.lift(event.slot(), event.time_msec()),
            InputEvent::TouchFrame { .. } => {
                let state = &mut self.state;
                if let Some(touch) = state.seat.get_touch() {
                    touch.frame(state);
                }
            }
            InputEvent::TouchCancel { event } => {
                let slots: Vec<TouchSlot> = self.state.touch.contacts.keys().copied().collect();
                for slot in slots {
                    if matches!(self.state.touch.contacts.get(&slot), Some(Contact::Pointer)) {
                        self.lift(slot, event.time_msec());
                    }
                }
                self.state.touch.contacts.clear();
                let state = &mut self.state;
                if let Some(touch) = state.seat.get_touch() {
                    touch.cancel(state);
                }
            }
            InputEvent::TabletToolProximity { event } => self.pen_proximity::<B>(&event),
            InputEvent::TabletToolAxis { event } => self.pen_moved::<B>(&event),
            InputEvent::TabletToolTip { event } => self.pen_tip::<B>(&event),
            InputEvent::TabletToolButton { event } => self.pen_button::<B>(&event),
            _ => {}
        }
    }

    fn pen_target(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.state
            .client_under(location)
            .filter(|(surface, _)| self.state.pen_reaches(surface))
    }

    fn pen_proximity<B: InputBackend>(&mut self, event: &B::TabletToolProximityEvent) {
        let location = mapped(&mut self.state, event);
        let time = event.time_msec();
        let state = &mut self.state;
        let tablet_seat = state.seat.tablet_seat();
        let display = state.display_handle.clone();
        let tablet =
            tablet_seat.add_tablet::<State>(&display, &TabletDescriptor::from(&event.device()));
        let tool = tablet_seat.add_tool::<State>(state, &display, &event.tool());
        if event.state() == smithay::backend::input::ProximityState::Out {
            tool.proximity_out(time);
            state.touch.pen_focus = None;
            return;
        }
        match self.pen_target(location) {
            Some(focus) => {
                tool.proximity_in(
                    location,
                    focus.clone(),
                    &tablet,
                    SERIAL_COUNTER.next_serial(),
                    time,
                );
                self.state.touch.pen_focus = Some(focus);
            }
            None => self.move_pointer_to(location, time),
        }
    }

    fn pen_moved<B: InputBackend>(&mut self, event: &B::TabletToolAxisEvent) {
        let location = mapped(&mut self.state, event);
        let time = event.time_msec();
        let tablet_seat = self.state.seat.tablet_seat();
        let tool = tablet_seat.get_tool(&event.tool());
        let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&event.device()));
        let focus = match self.state.touch.pen_press {
            Some(PenPress::Client) => self
                .state
                .touch
                .pen_focus
                .clone()
                .filter(|(surface, _)| surface.is_alive()),
            Some(PenPress::Pointer) => None,
            None => self.pen_target(location),
        };
        match (tool, tablet) {
            (Some(tool), Some(tablet))
                if self.state.touch.pen_press != Some(PenPress::Pointer)
                    && (focus.is_some() || self.state.touch.pen_focus.is_some()) =>
            {
                if event.pressure_has_changed() {
                    tool.pressure(event.pressure());
                }
                if event.tilt_has_changed() {
                    tool.tilt(event.tilt());
                }
                self.state.touch.pen_focus.clone_from(&focus);
                tool.motion(location, focus, &tablet, SERIAL_COUNTER.next_serial(), time);
            }
            _ => self.move_pointer_to(location, time),
        }
    }

    fn pen_tip<B: InputBackend>(&mut self, event: &B::TabletToolTipEvent) {
        let time = event.time_msec();
        let tool = self.state.seat.tablet_seat().get_tool(&event.tool());
        let touch = &mut self.state.touch;
        let press = match event.tip_state() {
            TabletToolTipState::Down if touch.pen_press.is_none() => {
                let press = match (&tool, &touch.pen_focus) {
                    (Some(_), Some(_)) => PenPress::Client,
                    _ => PenPress::Pointer,
                };
                touch.pen_press = Some(press);
                Some((press, true))
            }
            TabletToolTipState::Down => None,
            TabletToolTipState::Up => touch.pen_press.take().map(|press| (press, false)),
        };
        match (press, tool) {
            (Some((PenPress::Client, true)), Some(tool)) => {
                tool.tip_down(SERIAL_COUNTER.next_serial(), time);
            }
            (Some((PenPress::Client, false)), Some(tool)) => tool.tip_up(time),
            (Some((PenPress::Pointer, down)), _) => self.press_button(BTN_LEFT, down, time),
            _ => {}
        }
    }

    fn pen_button<B: InputBackend>(&mut self, event: &B::TabletToolButtonEvent) {
        let time = event.time_msec();
        let pressed = event.button_state() == ButtonState::Pressed;
        match self.state.seat.tablet_seat().get_tool(&event.tool()) {
            Some(tool) if self.state.touch.pen_focus.is_some() => {
                tool.button(
                    event.button(),
                    event.button_state(),
                    SERIAL_COUNTER.next_serial(),
                    time,
                );
            }
            _ => self.press_button(crate::input::BTN_RIGHT, pressed, time),
        }
    }
}

impl GlobalDispatch<ZwpTabletManagerV2, (), State> for State {
    fn bind(
        state: &mut State,
        handle: &DisplayHandle,
        client: &Client,
        resource: New<ZwpTabletManagerV2>,
        global_data: &(),
        data_init: &mut DataInit<'_, State>,
    ) {
        state.touch.tablet_clients.retain(|known| {
            handle
                .backend_handle()
                .get_client_data(known.clone())
                .is_ok()
        });
        state.touch.tablet_clients.push(client.id());
        <TabletManagerState as GlobalDispatch<ZwpTabletManagerV2, (), State>>::bind(
            state,
            handle,
            client,
            resource,
            global_data,
            data_init,
        );
    }
}

smithay::reexports::wayland_server::delegate_dispatch!(State: [ZwpTabletManagerV2: ()] => TabletManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [ZwpTabletSeatV2: TabletSeatUserData] => TabletManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [ZwpTabletToolV2: TabletToolUserData] => TabletManagerState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [ZwpTabletV2: TabletUserData] => TabletManagerState);

pub(crate) fn create_global(handle: &DisplayHandle) -> TabletManagerState {
    TabletManagerState::new::<State>(handle)
}

impl State {
    pub(crate) fn take_over_touch(&mut self, serial: Serial, surface: &WlSurface) -> bool {
        let Some(slot) = self.touch_serial_of(serial, surface) else {
            return false;
        };
        if let Some(touch) = self.seat.get_touch() {
            touch.cancel(self);
        }
        self.touch.contacts.clear();
        self.touch.contacts.insert(slot, Contact::Pointer);
        self.pointer_location = self.touch.location;
        true
    }
}
