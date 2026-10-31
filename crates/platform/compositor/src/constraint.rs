use smithay::input::pointer::{PointerHandle, RelativeMotionEvent};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::RegionAttributes;
use smithay::wayland::pointer_constraints::{
    PointerConstraint, PointerConstraintsHandler, with_pointer_constraint,
};

use crate::state::{Headless, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Locked,
    Confined,
}

fn within(region: Option<&RegionAttributes>, point: Point<f64, Logical>) -> bool {
    region.is_none_or(|region| region.contains(point.to_i32_floor()))
}

impl State {
    fn may_constrain(&self, surface: &WlSurface) -> bool {
        let focused = self.focused();
        focused.is_some() && self.toplevel_of(surface) == focused
    }

    fn held_constraint(&self, surface: &WlSurface, pointer: &PointerHandle<State>) -> Option<Held> {
        let allowed = self.may_constrain(surface);
        with_pointer_constraint(surface, pointer, |constraint| {
            let constraint = constraint?;
            if !constraint.is_active() {
                return None;
            }
            if !allowed {
                constraint.deactivate();
                return None;
            }
            Some(match &*constraint {
                PointerConstraint::Locked(_) => Held::Locked,
                PointerConstraint::Confined(_) => Held::Confined,
            })
        })
    }

    pub(crate) fn activate_pointer_constraint(&mut self) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let Some((surface, origin)) = self.surface_under(self.pointer_location) else {
            return;
        };
        if pointer.current_focus().as_ref() != Some(&surface) || !self.may_constrain(&surface) {
            return;
        }
        let local = self.pointer_location - origin;
        with_pointer_constraint(&surface, &pointer, |constraint| {
            if let Some(constraint) = constraint
                && !constraint.is_active()
                && within(constraint.region(), local)
            {
                constraint.activate();
            }
        });
    }

    fn confined_target(
        &self,
        surface: &WlSurface,
        pointer: &PointerHandle<State>,
        from: Point<f64, Logical>,
        delta: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let allowed = |to: Point<f64, Logical>| {
            self.surface_under(to)
                .filter(|(under, _)| under == surface)
                .is_some_and(|(_, origin)| {
                    with_pointer_constraint(surface, pointer, |constraint| {
                        within(
                            constraint.and_then(|held| held.region().cloned()).as_ref(),
                            to - origin,
                        )
                    })
                })
        };
        [
            from + delta,
            from + Point::from((delta.x, 0.0)),
            from + Point::from((0.0, delta.y)),
        ]
        .into_iter()
        .find(|to| allowed(*to))
        .unwrap_or(from)
    }
}

impl Headless {
    pub fn move_pointer_by(
        &mut self,
        delta: Point<f64, Logical>,
        unaccelerated: Point<f64, Logical>,
        utime: u64,
    ) {
        let time = u32::try_from((utime / 1000) & u64::from(u32::MAX)).unwrap_or_default();
        let from = self.state.pointer_location;
        let Some(pointer) = self.state.seat.get_pointer() else {
            self.move_pointer_to(from + delta, time);
            return;
        };
        let under = self.state.surface_under(from);
        pointer.relative_motion(
            &mut self.state,
            under.clone(),
            &RelativeMotionEvent {
                delta,
                delta_unaccel: unaccelerated,
                utime,
            },
        );
        let held = under
            .as_ref()
            .filter(|(surface, _)| pointer.current_focus().as_ref() == Some(surface))
            .and_then(|(surface, _)| {
                Some((surface, self.state.held_constraint(surface, &pointer)?))
            });
        let to = match held {
            Some((_, Held::Locked)) => {
                pointer.frame(&mut self.state);
                self.state.note_activity();
                return;
            }
            Some((surface, Held::Confined)) => {
                self.state.confined_target(surface, &pointer, from, delta)
            }
            None => from + delta,
        };
        self.move_pointer_to(to, time);
        self.state.activate_pointer_constraint();
    }
}

impl PointerConstraintsHandler for State {
    fn new_constraint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        self.activate_pointer_constraint();
    }

    fn cursor_position_hint(
        &mut self,
        surface: &WlSurface,
        pointer: &PointerHandle<Self>,
        location: Point<f64, Logical>,
    ) {
        if self.held_constraint(surface, pointer) != Some(Held::Locked) {
            return;
        }
        if let Some((_, origin)) = self
            .surface_under(self.pointer_location)
            .filter(|(under, _)| under == surface)
        {
            self.pointer_location = origin + location;
            self.needs_redraw = true;
        }
    }
}

smithay::delegate_pointer_constraints!(State);
smithay::delegate_relative_pointer!(State);
