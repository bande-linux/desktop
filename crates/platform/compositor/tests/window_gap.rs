#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use bande_compositor::{Compositor, OutputConfig};
use bande_config::Config;
use bande_test_wayland::keys::{
    BTN_LEFT, KEY_DOWN, KEY_ENTER, KEY_ESC, KEY_LEFT, KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_RIGHT,
    KEY_UP,
};
use bande_test_wayland::{Anchor, Bar, TestClient, chord, headless, roundtrip};

type Frame = (i32, i32, i32, i32);

const THICKNESS: i32 = 64;
const FRONT: usize = 1;
const BEHIND: usize = 0;

fn config(floating: bool, gap: i32) -> Config {
    let mut config = Config::default();
    config.bar.floating = floating;
    config.bar.gap = gap.unsigned_abs();
    config
}

fn bar_anchor(edge: Anchor) -> Anchor {
    if edge.intersects(Anchor::Left | Anchor::Right) {
        edge | Anchor::Top | Anchor::Bottom
    } else {
        edge | Anchor::Left | Anchor::Right
    }
}

struct Desktop {
    server: Compositor,
    shell: TestClient,
    bar: Bar,
    behind: TestClient,
    front: TestClient,
}

impl Desktop {
    fn new(physical: (i32, i32), scale: f64, edge: Anchor, config: Config) -> Self {
        let mut server = headless(physical.0, physical.1);
        server.set_output_scale(scale);
        let gap = match config.bar.floating {
            true => config.bar.gap.cast_signed(),
            false => 0,
        };
        server.set_display_config(config);
        server.set_output_scale(scale);
        let mut shell = TestClient::connect_as_shell(&mut server);
        let bar = shell.show_bar(&mut server, bar_anchor(edge), THICKNESS + 2 * gap);
        let behind = TestClient::window(&mut server, "org.example.Behind");
        let front = TestClient::window(&mut server, "org.example.Front");
        let mut desktop = Self {
            server,
            shell,
            bar,
            behind,
            front,
        };
        desktop.settle();
        desktop
    }

    fn settle(&mut self) {
        roundtrip(&mut self.server, &mut [&mut self.behind, &mut self.front]);
    }

    fn press(&mut self, keys: &[u32]) {
        chord(&mut self.server, keys);
        self.settle();
    }

    fn place(&mut self, key: u32) -> Frame {
        self.press(&[KEY_LEFTMETA, key]);
        self.press(&[KEY_ESC]);
        self.front_frame()
    }

    fn front_frame(&mut self) -> Frame {
        self.front.fill_configured_size(&mut self.server, 0);
        self.frame(FRONT)
    }

    fn frame(&self, index: usize) -> Frame {
        let frame = self.server.windows()[index].geometry().expect("mapped");
        (frame.loc.x, frame.loc.y, frame.size.w, frame.size.h)
    }

    fn tile(&self, index: usize) -> Frame {
        let tile = self.server.windows()[index].tile().expect("tiled");
        (tile.loc.x, tile.loc.y, tile.size.w, tile.size.h)
    }

    fn both_halves(&mut self) -> [Frame; 2] {
        self.press(&[KEY_LEFTMETA, KEY_LEFT]);
        self.press(&[KEY_ENTER]);
        self.behind.fill_configured_size(&mut self.server, 0);
        self.front.fill_configured_size(&mut self.server, 0);
        self.tiles()
    }

    fn tiles(&self) -> [Frame; 2] {
        let mut tiles = [self.tile(BEHIND), self.tile(FRONT)];
        tiles.sort_unstable();
        tiles
    }
}

fn inside(output: (i32, i32), edge: Anchor, gap: i32) -> Frame {
    let from = |side| match edge == side {
        true => gap + THICKNESS + gap,
        false => gap,
    };
    let (left, top) = (from(Anchor::Left), from(Anchor::Top));
    (
        left,
        top,
        output.0 - left - from(Anchor::Right),
        output.1 - top - from(Anchor::Bottom),
    )
}

#[test]
fn every_gap_around_and_between_placed_windows_is_the_taskbars() {
    let outputs = [
        ((1200, 900), 1.0, (1200, 900)),
        ((1200, 900), 1.5, (800, 600)),
        ((1003, 701), 1.0, (1003, 701)),
    ];
    for (physical, scale, output) in outputs {
        for edge in [Anchor::Left, Anchor::Right, Anchor::Top, Anchor::Bottom] {
            for (floating, spacing) in [(true, 0), (true, 8), (true, 16), (true, 24), (false, 24)] {
                let gap = if floating { spacing } else { 0 };
                let case = format!("{output:?} at {scale}, {edge:?}, gap {spacing}, {floating}");
                let mut desktop = Desktop::new(physical, scale, edge, config(floating, spacing));
                let area = inside(output, edge, gap);
                let (x, y, width, height) = area;
                let (half, rest) = ((width - gap) / 2, width - gap - (width - gap) / 2);
                let (upper, lower) = ((height - gap) / 2, height - gap - (height - gap) / 2);
                let beside = x + half + gap;
                let below = y + upper + gap;

                assert_eq!(desktop.place(KEY_LEFT), (x, y, half, height), "{case}");
                assert_eq!(desktop.place(KEY_UP), (x, y, half, upper), "{case}");
                assert_eq!(
                    desktop.place(KEY_RIGHT),
                    (beside, y, rest, height),
                    "{case}"
                );
                assert_eq!(
                    desktop.place(KEY_DOWN),
                    (beside, below, rest, lower),
                    "{case}"
                );

                desktop.front.maximize(&mut desktop.server, 0);
                assert_eq!(desktop.front_frame(), area, "{case}");

                desktop.front.make_full_screen(&mut desktop.server, 0);
                assert_eq!(desktop.front_frame(), (0, 0, output.0, output.1), "{case}");
            }
        }
    }
}

#[test]
fn neighbours_keep_the_gap_while_filled_dragged_and_respaced() {
    let mut desktop = Desktop::new((1200, 900), 1.0, Anchor::Left, config(true, 8));
    let halves = desktop.both_halves();
    assert_eq!(halves, [(80, 8, 552, 884), (640, 8, 552, 884)]);

    desktop.server.move_pointer_to((636.0, 450.0).into(), 0);
    desktop.server.press_button(BTN_LEFT, true, 0);
    desktop.server.move_pointer_to((736.0, 450.0).into(), 10);
    desktop.server.press_button(BTN_LEFT, false, 20);
    desktop.settle();
    assert_eq!(desktop.tiles(), [(80, 8, 652, 884), (740, 8, 452, 884)]);

    desktop.server.set_display_config(config(true, 24));
    assert_eq!(desktop.tiles(), [(80, 24, 536, 852), (640, 24, 536, 852)]);
}

#[test]
fn a_live_change_lays_out_a_maximized_window_again_and_leaves_a_floating_one() {
    let mut desktop = Desktop::new((1200, 900), 1.0, Anchor::Bottom, config(true, 0));
    let floating = desktop.frame(BEHIND);
    desktop.front.maximize(&mut desktop.server, 0);
    assert_eq!(desktop.front_frame(), (0, 0, 1200, 836));
    desktop.server.set_display_config(config(true, 16));
    assert_eq!(desktop.front_frame(), (16, 16, 1168, 820));
    desktop.server.set_display_config(config(false, 16));
    assert_eq!(desktop.front_frame(), (0, 0, 1200, 836));
    assert_eq!(desktop.frame(BEHIND), floating);
}

#[test]
fn a_maximized_window_leaving_full_screen_stays_clear_of_the_bar_that_returns() {
    let mut desktop = Desktop::new((1200, 900), 1.0, Anchor::Left, config(true, 8));
    desktop.front.maximize(&mut desktop.server, 0);
    let area = desktop.front_frame();
    desktop.front.make_full_screen(&mut desktop.server, 0);
    assert_eq!(desktop.front_frame(), (0, 0, 1200, 900));
    desktop
        .shell
        .set_bar_zone(&mut desktop.server, &desktop.bar, 0);
    desktop.settle();

    desktop.front.leave_full_screen(&mut desktop.server, 0);
    desktop
        .shell
        .set_bar_zone(&mut desktop.server, &desktop.bar, THICKNESS + 16);
    assert_eq!(desktop.front_frame(), area);
}

#[test]
fn every_display_has_the_gap_on_its_own_edges() {
    let mut desktop = Desktop::new((1200, 900), 1.0, Anchor::Left, config(true, 16));
    desktop.server.add_output(OutputConfig::virtual_display(
        "HEADLESS-2",
        (1000, 800).into(),
    ));
    assert_eq!(desktop.place(KEY_RIGHT), (648, 16, 536, 868));
    desktop.press(&[KEY_LEFTSHIFT, KEY_LEFTMETA, KEY_RIGHT]);
    assert_eq!(desktop.front_frame(), (1708, 16, 476, 768));
    assert_eq!(desktop.place(KEY_LEFT), (1216, 16, 476, 768));
    desktop.front.maximize(&mut desktop.server, 0);
    assert_eq!(desktop.front_frame(), (1216, 16, 968, 768));
}

#[test]
fn a_display_too_small_for_the_gap_still_places_windows() {
    let mut desktop = Desktop::new((120, 100), 1.0, Anchor::Left, config(true, 24));
    for key in [KEY_LEFT, KEY_RIGHT, KEY_DOWN] {
        desktop.press(&[KEY_LEFTMETA, key]);
    }
    desktop.front.maximize(&mut desktop.server, 0);
}
