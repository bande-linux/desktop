use unbound_toolkit::{Color, Palette, Scheme};

fn relative_luminance(color: Color) -> f64 {
    let linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast(a: Color, b: Color) -> f64 {
    let (a, b) = (relative_luminance(a), relative_luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn labels_meet_wcag_aa_text_contrast_on_every_surface() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        let palette = Palette::for_scheme(scheme);
        for surface in [palette.surface_background, palette.surface_elevated] {
            for label in [palette.label_primary, palette.label_muted] {
                let ratio = contrast(label, surface);
                assert!(
                    ratio >= 4.5,
                    "{scheme:?}: {label:?} on {surface:?} is {ratio:.2}:1"
                );
            }
        }
    }
}

#[test]
fn accent_meets_wcag_aa_non_text_contrast_on_every_surface() {
    for scheme in [Scheme::Light, Scheme::Dark] {
        let palette = Palette::for_scheme(scheme);
        for surface in [palette.surface_background, palette.surface_elevated] {
            let ratio = contrast(palette.accent_color, surface);
            assert!(
                ratio >= 3.0,
                "{scheme:?}: accent on {surface:?} is {ratio:.2}:1"
            );
        }
    }
}
