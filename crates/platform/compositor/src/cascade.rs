use smithay::reexports::wayland_server::Resource;
use smithay::utils::{Logical, Point, Size};

use crate::state::{Rect, State, centered};

const CASCADE_STEP: i32 = 28;

fn step(area: Rect, size: Size<i32, Logical>, from: Point<i32, Logical>) -> Point<i32, Logical> {
    let mut at = from + Point::from((CASCADE_STEP, CASCADE_STEP));
    if at.x + size.w > area.location.x + area.size.w {
        at.x = area.location.x;
    }
    if at.y + size.h > area.location.y + area.size.h {
        at.y = area.location.y;
    }
    at
}

pub(crate) fn cascaded(
    area: Rect,
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
        let owner = self.windows[index].surface().client();
        self.windows
            .iter()
            .enumerate()
            .filter(|(other, window)| {
                *other != index && self.present(window) && window.surface().client() == owner
            })
            .map(|(_, window)| window.location)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect {
            location: (0, 30).into(),
            size: (1000, 700).into(),
        }
    }

    #[test]
    fn a_first_window_is_centred_and_the_next_steps_down_and_right_from_the_frontmost() {
        let size = (400, 300).into();
        assert_eq!(cascaded(area(), size, &[]), (300, 230).into());
        assert_eq!(
            cascaded(area(), size, &[(40, 60).into(), (100, 130).into()]),
            (128, 158).into()
        );
    }

    #[test]
    fn a_window_that_would_leave_the_work_area_wraps_and_steps_past_its_apps_windows() {
        let size = (400, 300).into();
        assert_eq!(cascaded(area(), size, &[(590, 420).into()]), (0, 30).into());
        assert_eq!(
            cascaded(
                area(),
                size,
                &[(0, 30).into(), (28, 58).into(), (590, 420).into()]
            ),
            (56, 86).into()
        );
    }
}
