use bande_config::CornerAction;
use bande_protocols::server::bande_shell_v1::Panel;
use smithay::utils::{Logical, Point};

use crate::state::State;

const REACH: f64 = 1.0;
const RELEASE: f64 = 32.0;
const BEYOND: f64 = 8.0;

impl State {
    pub(crate) fn hot_corner(&mut self, location: Point<f64, Logical>) {
        if let Some(fired) = self.hot_corner_fired {
            if (location.x - fired.x).abs() <= RELEASE && (location.y - fired.y).abs() <= RELEASE {
                return;
            }
            self.hot_corner_fired = None;
        }
        let Some(screen) = self.screen_at(location) else {
            return;
        };
        let rect = self.screen_rect(screen);
        let (left, top) = (f64::from(rect.loc.x), f64::from(rect.loc.y));
        let right = left + f64::from(rect.size.w - 1);
        let bottom = top + f64::from(rect.size.h - 1);
        let side = |at: f64, low: f64, high: f64| {
            if at <= low + REACH {
                Some((0, -BEYOND))
            } else if at >= high - REACH {
                Some((1, BEYOND))
            } else {
                None
            }
        };
        let (Some((column, dx)), Some((row, dy))) =
            (side(location.x, left, right), side(location.y, top, bottom))
        else {
            return;
        };
        let beyond = self.nearest_on_screens((location.x + dx, location.y + dy).into());
        if self.screen_at(beyond) != Some(screen) || self.is_locked() {
            return;
        }
        let action = self.display_config.hot_corners.actions[row * 2 + column];
        if action == CornerAction::Nothing {
            return;
        }
        self.hot_corner_fired = Some(location);
        self.close_menus();
        match action {
            CornerAction::Nothing => {}
            CornerAction::MissionControl => self.toggle_overview(None),
            CornerAction::AppWindows => self.show_app_windows(),
            CornerAction::Desktop => self.toggle_desktop(),
            CornerAction::NotificationCenter => self.toggle_panel(Panel::NotificationCenter),
            CornerAction::Search => self.show_launcher(),
            CornerAction::LockScreen => self.lock_screen_requested = true,
        }
    }
}
