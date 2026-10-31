#![allow(clippy::expect_used)]

mod support;

use unbound_bar::pairing::{Question, pairing_scene};
use unbound_toolkit::{CpuRenderer, Palette, Scheme};

use support::check_golden;

#[test]
fn the_shell_asks_to_confirm_a_code_or_type_a_pin_while_pairing() {
    for (question, name) in [
        (
            Question::Confirm {
                device: "Phone".to_owned(),
                code: "123456".to_owned(),
            },
            "pairing-confirm-light",
        ),
        (
            Question::Pin {
                device: "Keyboard".to_owned(),
            },
            "pairing-pin-light",
        ),
    ] {
        let scene = pairing_scene(&question, Palette::for_scheme(Scheme::Light));
        let size = (scene.width, scene.height);
        let mut rgba = vec![0; (size.0 * size.1 * 4) as usize];
        CpuRenderer::new()
            .render_into(&scene, 1.0, &mut rgba)
            .expect("frame");
        check_golden(name, size, rgba);
    }
}
