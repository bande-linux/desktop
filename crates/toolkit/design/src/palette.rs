use crate::{Color, TextSize};

const LIGHT_NEUTRAL_HUE: f64 = 80.0;
const DARK_NEUTRAL_HUE: f64 = 262.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scheme {
    #[default]
    Light,
    Dark,
}

impl Scheme {
    const fn neutral_hue(self) -> f64 {
        match self {
            Self::Light => LIGHT_NEUTRAL_HUE,
            Self::Dark => DARK_NEUTRAL_HUE,
        }
    }
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
    dark_hue: f64,
    light: Tone,
    dark: Tone,
    dark_fill: Tone,
}

impl Hued {
    fn color(&self, scheme: Scheme) -> Color {
        match scheme {
            Scheme::Light => self.paint(self.hue, &self.light),
            Scheme::Dark => self.paint(self.dark_hue, &self.dark),
        }
    }

    fn fill(&self, scheme: Scheme) -> Color {
        match scheme {
            Scheme::Light => self.paint(self.hue, &self.light),
            Scheme::Dark => self.paint(self.dark_hue, &self.dark_fill),
        }
    }

    fn paint(&self, hue: f64, tone: &Tone) -> Color {
        Color::oklch(tone.lightness, tone.chroma, hue)
    }
}

const fn hued(hue: f64, light: Tone, dark: Tone, dark_fill: Tone) -> Hued {
    Hued {
        hue,
        dark_hue: hue,
        light,
        dark,
        dark_fill,
    }
}

const fn signal(hue: f64, light: Tone, dark: Tone) -> Hued {
    let dark_fill = tone(dark.lightness, dark.chroma);
    hued(hue, light, dark, dark_fill)
}

const DANGER: Hued = hued(25.0, tone(0.57, 0.20), tone(0.70, 0.17), tone(0.58, 0.18));
const WARNING: Hued = signal(70.0, tone(0.70, 0.15), tone(0.80, 0.14));
const SUCCESS: Hued = signal(150.0, tone(0.56, 0.13), tone(0.74, 0.14));

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
            Self::Ultramarine => hued(272.0, tone(0.53, 0.20), tone(0.70, 0.15), tone(0.568, 0.16)),
            Self::Teal => hued(195.0, tone(0.55, 0.10), tone(0.74, 0.11), tone(0.545, 0.10)),
            Self::Moss => hued(140.0, tone(0.55, 0.12), tone(0.75, 0.13), tone(0.548, 0.12)),
            Self::Amber => hued(65.0, tone(0.63, 0.14), tone(0.80, 0.13), tone(0.74, 0.14)),
            Self::Ember => hued(38.0, tone(0.58, 0.19), tone(0.72, 0.16), tone(0.577, 0.17)),
            Self::Rose => hued(5.0, tone(0.56, 0.19), tone(0.73, 0.15), tone(0.580, 0.17)),
            Self::Plum => hued(320.0, tone(0.52, 0.16), tone(0.72, 0.13), tone(0.577, 0.15)),
            Self::Graphite => Hued {
                dark_hue: DARK_NEUTRAL_HUE,
                ..hued(
                    LIGHT_NEUTRAL_HUE,
                    tone(0.30, 0.01),
                    tone(0.90, 0.01),
                    tone(0.86, 0.01),
                )
            },
        }
    }

    pub fn color(self, scheme: Scheme) -> Color {
        self.hued().color(scheme)
    }

    pub fn fill(self, scheme: Scheme) -> Color {
        self.hued().fill(scheme)
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
    pub rim: Color,
    pub edge: Color,
    pub ink: Color,
    pub ink2: Color,
    pub ink3: Color,
    pub deep: Color,
    pub accent: Color,
    pub accent_text: Color,
    pub on_accent: Color,
    pub accent_soft: Color,
    pub focus: Color,
    pub danger: Color,
    pub danger_text: Color,
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
        let hue = scheme.neutral_hue();
        let neutral = |lightness: f64, chroma: f64| Color::oklch(lightness, chroma, hue);
        let increased = contrast == Contrast::Increased;
        let (neutrals, scrim, soft) = match scheme {
            Scheme::Light => (
                [
                    neutral(0.938, 0.005),
                    neutral(0.988, 0.0025),
                    neutral(1.0, 0.0),
                    neutral(0.905, 0.007),
                    neutral(0.86, 0.008),
                    neutral(0.86, 0.008),
                    neutral(if increased { 0.62 } else { 0.78 }, 0.008),
                    neutral(0.235, 0.008),
                    neutral(0.46, 0.010),
                    neutral(0.60, 0.010),
                    neutral(0.235, 0.008),
                ],
                0.18,
                0.84,
            ),
            Scheme::Dark => (
                [
                    neutral(0.255, 0.013),
                    neutral(0.285, 0.012),
                    neutral(0.335, 0.011),
                    neutral(0.325, 0.012),
                    neutral(0.365, 0.012),
                    neutral(0.42, 0.012),
                    neutral(if increased { 0.58 } else { 0.47 }, 0.012),
                    neutral(if increased { 0.965 } else { 0.925 }, 0.006),
                    neutral(if increased { 0.86 } else { 0.80 }, 0.012),
                    neutral(if increased { 0.74 } else { 0.655 }, 0.012),
                    neutral(0.215, 0.015),
                ],
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
            rim,
            edge,
            ink,
            ink2,
            ink3,
            deep,
        ] = neutrals;
        let signal = accent.color(scheme);
        let fill = accent.fill(scheme);
        let danger = DANGER.fill(scheme);
        let legible_on = |fill: Color| {
            [Color::WHITE, deep]
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
            rim,
            edge,
            ink,
            ink2,
            ink3,
            deep,
            accent: fill,
            accent_text: signal,
            on_accent: legible_on(fill),
            accent_soft: signal.mix(sheet, soft),
            focus: signal,
            danger,
            danger_text: DANGER.color(scheme),
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
    pub frame_tint: Option<Color>,
    pub text_size: TextSize,
}

const FRAME_TINT: f32 = 0.06;

impl Appearance {
    pub fn palette(self) -> Palette {
        let mut palette = Palette::generate(self.scheme, self.accent, self.contrast);
        if let Some(tint) = self.frame_tint {
            palette.chassis = palette.chassis.mix(tint, FRAME_TINT);
        }
        palette
    }
}
