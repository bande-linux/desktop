use bande_design::{Accent, Appearance, Color, Contrast, Palette, Scheme, apca, wcag};

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

fn luminance(color: Color) -> f64 {
    (wcag(color, Color::BLACK) - 1.0) * 0.05
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
        (dark.chassis, 0x1F2329),
        (dark.sheet, 0x272A30),
        (dark.overlay, 0x34373C),
        (dark.well, 0x31343A),
        (dark.hairline, 0x3B3F45),
        (dark.rim, 0x494D54),
        (dark.edge, 0x575B62),
        (dark.ink, 0xE4E6EA),
        (dark.ink2, 0xB9BEC6),
        (dark.ink3, 0x8D9198),
        (dark.deep, 0x161A21),
        (dark.danger, 0xCF4040),
        (dark.danger_text, 0xF66D67),
        (dark.warning, 0xF7AC4D),
        (dark.success, 0x62C37A),
    ];
    for (generated, hex) in expected {
        assert_eq!(generated, Color::hex(hex), "expected #{hex:06X}");
    }
    let accents = [
        (Accent::Ultramarine, 0x4859DD, 0x7E96FB, 0x576CD4),
        (Accent::Teal, 0x008384, 0x3FC0C0, 0x008282),
        (Accent::Moss, 0x48823B, 0x80C271, 0x48823B),
        (Accent::Amber, 0xC27405, 0xF7AB5D, 0xE7963D),
        (Accent::Ember, 0xD24100, 0xF77C56, 0xC94A1F),
        (Accent::Rose, 0xC83065, 0xF47C9B, 0xC7436D),
        (Accent::Plum, 0x9043A1, 0xC889D7, 0x9F58B0),
        (Accent::Graphite, 0x302D28, 0xDADEE5, 0xCDD1D8),
    ];
    for (accent, light, dark, dark_fill) in accents {
        assert_eq!(accent.color(Scheme::Light), Color::hex(light), "{accent:?}");
        assert_eq!(accent.fill(Scheme::Light), Color::hex(light), "{accent:?}");
        assert_eq!(accent.color(Scheme::Dark), Color::hex(dark), "{accent:?}");
        assert_eq!(
            accent.fill(Scheme::Dark),
            Color::hex(dark_fill),
            "{accent:?}"
        );
    }
}

#[test]
fn text_roles_stay_legible_on_every_surface_in_every_palette() {
    for (name, palette) in every_palette() {
        let ink_floor = match palette.scheme {
            Scheme::Light => 90.0,
            Scheme::Dark => 75.0,
        };
        for surface in [palette.chassis, palette.sheet, palette.overlay] {
            for (role, text, min_wcag, min_apca) in [
                ("ink", palette.ink, 7.0, ink_floor),
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
fn dark_standard_ink_stays_under_the_glare_ceiling() {
    for accent in Accent::ALL {
        let palette = Palette::generate(Scheme::Dark, accent, Contrast::Standard);
        for surface in [palette.chassis, palette.sheet, palette.overlay] {
            let lc = apca(palette.ink, surface).abs();
            assert!(lc <= 90.0, "{accent:?}: ink on {surface:?} is Lc {lc:.0}");
        }
    }
}

#[test]
fn dark_surfaces_step_up_in_lightness() {
    for (name, palette) in every_palette().filter(|(_, palette)| palette.scheme == Scheme::Dark) {
        let steps = [
            palette.chassis,
            palette.sheet,
            palette.well,
            palette.overlay,
            palette.hairline,
            palette.rim,
            palette.edge,
        ]
        .map(luminance);
        for (lower, upper) in steps.iter().zip(&steps[1..]) {
            assert!(lower < upper, "{name}: {steps:?}");
        }
    }
}

#[test]
fn a_wallpaper_tint_shifts_only_the_frame_and_keeps_its_text_legible() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        for tint in [
            Color::hex(0xFF0000),
            Color::hex(0x0000FF),
            Color::BLACK,
            Color::WHITE,
        ] {
            let plain = Appearance {
                scheme,
                ..Appearance::default()
            }
            .palette();
            let tinted = Appearance {
                scheme,
                frame_tint: Some(tint),
                ..Appearance::default()
            }
            .palette();
            assert_eq!(
                Palette {
                    chassis: plain.chassis,
                    ..tinted
                },
                plain
            );
            let moved = |from: u8, to: u8| i32::from(to) - i32::from(from);
            for (before, after, toward) in [
                (plain.chassis.r, tinted.chassis.r, tint.r),
                (plain.chassis.g, tinted.chassis.g, tint.g),
                (plain.chassis.b, tinted.chassis.b, tint.b),
            ] {
                assert!(moved(before, after).abs() <= 16);
                assert!(moved(before, after) * moved(before, toward) >= 0);
            }
            assert!(
                wcag(tinted.ink2, tinted.chassis) >= 4.5,
                "{scheme:?} {tint:?}"
            );
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
            for (role, signal) in [
                ("accent text", palette.accent_text),
                ("focus", palette.focus),
                ("danger text", palette.danger_text),
            ] {
                let ratio = wcag(signal, surface);
                assert!(
                    ratio >= 3.0,
                    "{name}: {role} on {surface:?} is {ratio:.2}:1"
                );
            }
        }
        for surface in [palette.chassis, palette.sheet] {
            for (role, fill) in [("accent", palette.accent), ("danger", palette.danger)] {
                let ratio = wcag(fill, surface);
                assert!(
                    ratio >= 3.0,
                    "{name}: {role} on {surface:?} is {ratio:.2}:1"
                );
            }
        }
        let ratio = wcag(palette.on_accent, palette.accent);
        assert!(ratio >= 4.5, "{name}: text on the accent is {ratio:.2}:1");
        let ratio = wcag(palette.on_danger, palette.danger);
        assert!(ratio >= 4.5, "{name}: text on danger is {ratio:.2}:1");
    }
}

#[test]
fn dark_fills_are_dimmer_than_their_text_tone_and_carry_legible_text() {
    for accent in Accent::ALL {
        let palette = Palette::generate(Scheme::Dark, accent, Contrast::Standard);
        let glare = luminance(palette.accent);
        assert!(glare < luminance(palette.accent_text), "{accent:?}");
        match accent {
            Accent::Amber | Accent::Graphite => assert_eq!(palette.on_accent, palette.deep),
            _ => {
                assert!(glare <= 0.2, "{accent:?}: fill luminance {glare:.3}");
                assert_eq!(palette.on_accent, Color::WHITE, "{accent:?}");
            }
        }
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

#[test]
fn sidebar_selection_tones_differ_and_keep_ink_legible_in_every_palette() {
    for (name, palette) in every_palette() {
        let tones = [
            palette.sidebar_selection(true, false),
            palette.sidebar_selection(true, true),
            palette.sidebar_selection(false, false),
        ];
        for (index, tone) in tones.iter().enumerate() {
            assert_ne!(
                *tone, palette.chassis,
                "{name}: tone {index} is the chassis"
            );
            assert!(
                tones[index + 1..].iter().all(|other| other != tone),
                "{name}: tone {index} repeats"
            );
            let ratio = wcag(palette.ink, *tone);
            assert!(ratio >= 7.0, "{name}: ink on tone {index} is {ratio:.2}:1");
        }
    }
}
