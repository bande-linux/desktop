use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, Rectangle, Size};

use crate::shell::toplevel_names;
use crate::state::{State, centered};
use crate::window::Window;

const CASCADE_STEP: i32 = 28;

fn step(
    area: Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
    from: Point<i32, Logical>,
) -> Point<i32, Logical> {
    let mut at = from + Point::from((CASCADE_STEP, CASCADE_STEP));
    if at.x + size.w > area.loc.x + area.size.w {
        at.x = area.loc.x;
    }
    if at.y + size.h > area.loc.y + area.size.h {
        at.y = area.loc.y;
    }
    at
}

pub(crate) fn cascaded(
    area: Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
    siblings: &[Point<i32, Logical>],
) -> Point<i32, Logical> {
    let mut at = match siblings.last() {
        Some(frontmost) => step(area, size, *frontmost),
        None => centered(area, size),
    };
    for _ in 0..siblings.len() {
        if !siblings.contains(&at) {
            break;
        }
        at = step(area, size, at);
    }
    at
}

impl State {
    pub(crate) fn sibling_locations(&self, index: usize) -> Vec<Point<i32, Logical>> {
        let app = |window: &Window| {
            let app_id = toplevel_names(window.surface()).1;
            (app_id.is_empty().then(|| window.surface().client()), app_id)
        };
        let own = app(&self.windows[index]);
        let screen = self.screen_of(&self.windows[index]);
        self.windows
            .iter()
            .enumerate()
            .filter(|(other, window)| {
                *other != index
                    && self.present(window)
                    && self.screen_of(window) == screen
                    && app(window) == own
            })
            .map(|(_, window)| window.location)
            .collect()
    }
}
