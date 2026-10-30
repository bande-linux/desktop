use crate::Color;

const NEUTRAL_HUE: f64 = 80.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scheme {
    #[default]
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Contrast {
    #[default]
    Standard,
    Increased,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Accent {
    #[default]
    Ultramarine,
    Teal,
    Moss,
    Amber,
    Ember,
    Rose,
    Plum,
    Graphite,
}

struct Tone {
    lightness: f64,
    chroma: f64,
}

const fn tone(lightness: f64, chroma: f64) -> Tone {
    Tone { lightness, chroma }
}

struct Hued {
    hue: f64,
    light: Tone,
    dark: Tone,
}

impl Hued {
    fn color(&self, scheme: Scheme) -> Color {
        let tone = match scheme {
            Scheme::Light => &self.light,
            Scheme::Dark => &self.dark,
        };
        Color::oklch(tone.lightness, tone.chroma, self.hue)
    }
}

const fn hued(hue: f64, light: Tone, dark: Tone) -> Hued {
    Hued { hue, light, dark }
}

const DANGER: Hued = hued(25.0, tone(0.57, 0.20), tone(0.70, 0.17));
const WARNING: Hued = hued(70.0, tone(0.70, 0.15), tone(0.80, 0.14));
const SUCCESS: Hued = hued(150.0, tone(0.56, 0.13), tone(0.74, 0.14));

impl Accent {
    pub const ALL: [Self; 8] = [
        Self::Ultramarine,
        Self::Teal,
        Self::Moss,
        Self::Amber,
        Self::Ember,
        Self::Rose,
        Self::Plum,
        Self::Graphite,
    ];

    const fn hued(self) -> Hued {
        match self {
            Self::Ultramarine => hued(272.0, tone(0.53, 0.20), tone(0.70, 0.15)),
            Self::Teal => hued(195.0, tone(0.55, 0.10), tone(0.74, 0.11)),
            Self::Moss => hued(140.0, tone(0.55, 0.12), tone(0.75, 0.13)),
            Self::Amber => hued(65.0, tone(0.63, 0.14), tone(0.80, 0.13)),
            Self::Ember => hued(38.0, tone(0.58, 0.19), tone(0.72, 0.16)),
            Self::Rose => hued(5.0, tone(0.56, 0.19), tone(0.73, 0.15)),
            Self::Plum => hued(320.0, tone(0.52, 0.16), tone(0.72, 0.13)),
            Self::Graphite => hued(NEUTRAL_HUE, tone(0.30, 0.01), tone(0.90, 0.01)),
        }
    }

    pub fn color(self, scheme: Scheme) -> Color {
        self.hued().color(scheme)
    }

    pub fn nearest(color: Color) -> Self {
        let [_, a, b] = color.oklab();
        let distance = |candidate: Color| {
            let [_, ca, cb] = candidate.oklab();
            (ca - a).powi(2) + (cb - b).powi(2)
        };
        let closest = |accent: &Self| {
            distance(accent.color(Scheme::Light)).min(distance(accent.color(Scheme::Dark)))
        };
        Self::ALL
            .into_iter()
            .min_by(|a, b| closest(a).total_cmp(&closest(b)))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub scheme: Scheme,
    pub chassis: Color,
    pub sheet: Color,
    pub overlay: Color,
    pub well: Color,
    pub field: Color,
    pub raised: Color,
    pub hairline: Color,
    pub edge: Color,
    pub ink: Color,
    pub ink2: Color,
    pub ink3: Color,
    pub accent: Color,
    pub accent_text: Color,
    pub on_accent: Color,
    pub accent_soft: Color,
    pub focus: Color,
    pub danger: Color,
    pub on_danger: Color,
    pub warning: Color,
    pub success: Color,
    pub scrim: Color,
}

impl Palette {
    pub fn for_scheme(scheme: Scheme) -> Self {
        Self::generate(scheme, Accent::default(), Contrast::default())
    }

    pub fn generate(scheme: Scheme, accent: Accent, contrast: Contrast) -> Self {
        let neutral = |lightness: f64, chroma: f64| Color::oklch(lightness, chroma, NEUTRAL_HUE);
        let increased = contrast == Contrast::Increased;
        let (neutrals, dark_text, scrim, soft) = match scheme {
            Scheme::Light => (
                [
                    neutral(0.938, 0.005),
                    neutral(0.988, 0.0025),
                    neutral(1.0, 0.0),
                    neutral(0.905, 0.007),
                    neutral(0.86, 0.008),
                    neutral(if increased { 0.62 } else { 0.78 }, 0.008),
                    neutral(0.235, 0.008),
                    neutral(0.46, 0.010),
                    neutral(0.60, 0.010),
                ],
                neutral(0.235, 0.008),
                0.18,
                0.84,
            ),
            Scheme::Dark => (
                [
                    neutral(0.185, 0.006),
                    neutral(0.225, 0.006),
                    neutral(0.265, 0.006),
                    neutral(0.275, 0.006),
                    neutral(0.31, 0.006),
                    neutral(if increased { 0.52 } else { 0.41 }, 0.006),
                    neutral(0.955, 0.004),
                    neutral(0.78, 0.008),
                    neutral(0.64, 0.008),
                ],
                neutral(0.185, 0.006),
                0.40,
                0.78,
            ),
        };
        let [
            chassis,
            sheet,
            overlay,
            well,
            hairline,
            edge,
            ink,
            ink2,
            ink3,
        ] = neutrals;
        let signal = accent.color(scheme);
        let danger = DANGER.color(scheme);
        let legible_on = |fill: Color| {
            [Color::WHITE, dark_text]
                .into_iter()
                .max_by(|a, b| crate::wcag(*a, fill).total_cmp(&crate::wcag(*b, fill)))
                .unwrap_or(Color::WHITE)
        };
        Self {
            scheme,
            chassis,
            sheet,
            overlay,
            well,
            field: match scheme {
                Scheme::Light => overlay,
                Scheme::Dark => well,
            },
            raised: match scheme {
                Scheme::Light => overlay,
                Scheme::Dark => edge,
            },
            hairline,
            edge,
            ink,
            ink2,
            ink3,
            accent: signal,
            accent_text: signal,
            on_accent: legible_on(signal),
            accent_soft: signal.mix(sheet, soft),
            focus: signal,
            danger,
            on_danger: legible_on(danger),
            warning: WARNING.color(scheme),
            success: SUCCESS.color(scheme),
            scrim: Color::BLACK.with_opacity(scrim),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Appearance {
    pub scheme: Scheme,
    pub accent: Accent,
    pub contrast: Contrast,
    pub reduce_motion: bool,
}

impl Appearance {
    pub fn palette(self) -> Palette {
        Palette::generate(self.scheme, self.accent, self.contrast)
    }
}
