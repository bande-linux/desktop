#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use support::{SCALES, SCHEMES, Shot, appearance_portal, shot};
use unbound_toolkit_platform::FrameRegions;

const OVER_CLOSE: (i32, i32) = (24, 24);
const AWAY: (i32, i32) = (200, 120);

#[test]
fn the_window_frame_matches_its_goldens_in_both_schemes_at_1x_and_1_5x() {
    let _portal = appearance_portal();
    for (dark, scheme) in SCHEMES {
        for (scale, scale_name) in SCALES {
            for (state, pointer) in [("rest", AWAY), ("close-hovered", OVER_CLOSE)] {
                shot(
                    &format!("window-frame-{state}-{scheme}-{scale_name}"),
                    |_| None,
                    Shot {
                        title: "Documents",
                        scale,
                        dark,
                        pointer,
                        popups: 0,
                    },
                );
            }
        }
        shot(
            &format!("window-frame-sidebar-{scheme}-1x"),
            |window| {
                window.set_frame_regions(FrameRegions {
                    sidebar: 110.0,
                    toolbar: 0.0,
                });
                None
            },
            Shot {
                title: "Documents",
                scale: 1.0,
                dark,
                pointer: AWAY,
                popups: 0,
            },
        );
    }
}
