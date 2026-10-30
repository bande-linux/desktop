use unbound_design::{Accent, Color, Contrast, Palette, Scheme, apca, wcag};

fn every_palette() -> impl Iterator<Item = (String, Palette)> {
    [Scheme::Light, Scheme::Dark]
        .into_iter()
        .flat_map(|scheme| {
            Accent::ALL.into_iter().flat_map(move |accent| {
                [Contrast::Standard, Contrast::Increased]
                    .into_iter()
                    .map(move |contrast| {
                        (
                            format!("{scheme:?} {accent:?} {contrast:?}"),
                            Palette::generate(scheme, accent, contrast),
                        )
                    })
            })
        })
}

#[test]
fn the_generator_reproduces_the_adopted_datum_values() {
    let light = Palette::for_scheme(Scheme::Light);
    let dark = Palette::for_scheme(Scheme::Dark);
    let expected = [
        (light.chassis, 0xECEAE7),
        (light.sheet, 0xFCFBF9),
        (light.overlay, 0xFFFFFF),
        (light.well, 0xE2DFDB),
        (light.hairline, 0xD4D0CB),
        (light.edge, 0xBAB7B2),
        (light.ink, 0x201E1A),
        (light.ink2, 0x5B5752),
        (light.ink3, 0x83807A),
        (light.danger, 0xD42F34),
        (light.warning, 0xD98B09),
        (light.success, 0x2D8949),
        (dark.chassis, 0x141210),
        (dark.sheet, 0x1D1B19),
        (dark.overlay, 0x272522),
        (dark.well, 0x292724),
        (dark.hairline, 0x32302D),
        (dark.edge, 0x4C4A47),
        (dark.ink, 0xF1F0ED),
        (dark.ink2, 0xBAB7B2),
        (dark.ink3, 0x8F8C87),
        (dark.danger, 0xF66D67),
        (dark.warning, 0xF7AC4D),
        (dark.success, 0x62C37A),
    ];
    for (generated, hex) in expected {
        assert_eq!(generated, Color::hex(hex), "expected #{hex:06X}");
    }
    let accents = [
        (Accent::Ultramarine, 0x4859DD, 0x7E96FB),
        (Accent::Teal, 0x008384, 0x3FC0C0),
        (Accent::Moss, 0x48823B, 0x80C271),
        (Accent::Amber, 0xC27405, 0xF7AB5D),
        (Accent::Ember, 0xD24100, 0xF77C56),
        (Accent::Rose, 0xC83065, 0xF47C9B),
        (Accent::Plum, 0x9043A1, 0xC889D7),
        (Accent::Graphite, 0x302D28, 0xE1DDD7),
    ];
    for (accent, light, dark) in accents {
        assert_eq!(accent.color(Scheme::Light), Color::hex(light), "{accent:?}");
        assert_eq!(accent.color(Scheme::Dark), Color::hex(dark), "{accent:?}");
    }
}

#[test]
fn text_roles_stay_legible_on_every_surface_in_every_palette() {
    for (name, palette) in every_palette() {
        for surface in [palette.chassis, palette.sheet, palette.overlay] {
            for (role, text, min_wcag, min_apca) in [
                ("ink", palette.ink, 7.0, 90.0),
                ("ink2", palette.ink2, 4.5, 60.0),
                ("ink3", palette.ink3, 3.0, 30.0),
            ] {
                let ratio = wcag(text, surface);
                let lc = apca(text, surface).abs();
                assert!(
                    ratio >= min_wcag && lc >= min_apca,
                    "{name}: {role} on {surface:?} is {ratio:.2}:1, Lc {lc:.0}"
                );
            }
        }
    }
}

#[test]
fn control_boundaries_meet_their_contrast_floor() {
    for (name, palette) in every_palette() {
        let floor = if name.ends_with("Increased") {
            3.0
        } else {
            1.9
        };
        let ratio = wcag(palette.edge, palette.sheet);
        assert!(ratio >= floor, "{name}: edge on sheet is {ratio:.2}:1");
    }
}

#[test]
fn the_signal_reads_on_every_surface_and_carries_legible_text() {
    for (name, palette) in every_palette() {
        for surface in [palette.chassis, palette.sheet, palette.overlay] {
            for (role, signal) in [("accent", palette.accent), ("danger", palette.danger)] {
                let ratio = wcag(signal, surface);
                assert!(
                    ratio >= 3.0,
                    "{name}: {role} on {surface:?} is {ratio:.2}:1"
                );
            }
        }
        let ratio = wcag(palette.on_accent, palette.accent);
        assert!(ratio >= 4.5, "{name}: text on the accent is {ratio:.2}:1");
    }
}

#[test]
fn dark_selections_carry_dark_text() {
    for accent in Accent::ALL {
        let palette = Palette::generate(Scheme::Dark, accent, Contrast::Standard);
        assert_eq!(palette.on_accent, palette.chassis, "{accent:?}");
    }
}

#[test]
fn a_portal_accent_maps_to_the_nearest_datum_accent() {
    for accent in Accent::ALL {
        for scheme in [Scheme::Light, Scheme::Dark] {
            assert_eq!(Accent::nearest(accent.color(scheme)), accent);
        }
    }
    assert_eq!(Accent::nearest(Color::hex(0x2F6FEB)), Accent::Ultramarine);
    assert_eq!(Accent::nearest(Color::hex(0x3584E4)), Accent::Ultramarine);
    assert_eq!(Accent::nearest(Color::hex(0x2EC27E)), Accent::Moss);
    assert_eq!(Accent::nearest(Color::hex(0xE01B24)), Accent::Ember);
    assert_eq!(Accent::nearest(Color::hex(0x808080)), Accent::Graphite);
}
