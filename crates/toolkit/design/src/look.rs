use crate::{Color, Scheme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Style {
    #[default]
    Refined,
    WallpaperLit,
    Glass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Material {
    Opaque,
    Wallpaper,
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub material: Material,
    pub frame_opacity: f32,
    pub panel_opacity: f32,
}

const LIT_FRAME_OPACITY: f32 = 0.74;
const BACKDROP_CEILING: f64 = 0.16;
const BACKDROP_FLOOR: f64 = 0.60;
const GLASS_FRAME_OPACITY: f32 = 0.68;
const GLASS_PANEL_OPACITY: f32 = 0.72;
pub const BACKDROP_SATURATION: f32 = 1.35;

impl Style {
    pub const ALL: [Self; 3] = [Self::Refined, Self::WallpaperLit, Self::Glass];

    pub const fn keyword(self) -> &'static str {
        match self {
            Self::Refined => "refined",
            Self::WallpaperLit => "wallpaper-lit",
            Self::Glass => "glass",
        }
    }

    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|style| style.keyword() == keyword)
    }

    pub const fn look(self) -> Look {
        match self {
            Self::Refined => Look::SOLID,
            Self::WallpaperLit => Look {
                material: Material::Wallpaper,
                frame_opacity: LIT_FRAME_OPACITY,
                panel_opacity: 1.0,
            },
            Self::Glass => Look {
                material: Material::Live,
                frame_opacity: GLASS_FRAME_OPACITY,
                panel_opacity: GLASS_PANEL_OPACITY,
            },
        }
    }
}

impl Look {
    pub const SOLID: Self = Self {
        material: Material::Opaque,
        frame_opacity: 1.0,
        panel_opacity: 1.0,
    };
}

fn to_linear(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn to_encoded(value: f64) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

pub fn limit_backdrop(color: [u8; 3], scheme: Scheme) -> [u8; 3] {
    let linear = color.map(to_linear);
    let luminance = 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2];
    let limited = match scheme {
        Scheme::Dark if luminance > BACKDROP_CEILING => {
            linear.map(|channel| channel * BACKDROP_CEILING / luminance)
        }
        Scheme::Light if luminance < BACKDROP_FLOOR => {
            let toward_white = (BACKDROP_FLOOR - luminance) / (1.0 - luminance);
            linear.map(|channel| channel + (1.0 - channel) * toward_white)
        }
        _ => linear,
    };
    limited.map(to_encoded)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackdropTone {
    pub luminance: f32,
    pub floor: bool,
    pub saturation: f32,
}

pub fn backdrop_tone(scheme: Scheme) -> BackdropTone {
    match scheme {
        Scheme::Dark => BackdropTone {
            luminance: BACKDROP_CEILING as f32,
            floor: false,
            saturation: BACKDROP_SATURATION,
        },
        Scheme::Light => BackdropTone {
            luminance: BACKDROP_FLOOR as f32,
            floor: true,
            saturation: BACKDROP_SATURATION,
        },
    }
}

pub fn over_backdrop(frame: Color, backdrop: [u8; 3]) -> Color {
    let opacity = f32::from(frame.a) / 255.0;
    let mix = |top: u8, bottom: u8| {
        (f32::from(top) * opacity + f32::from(bottom) * (1.0 - opacity)).round() as u8
    };
    Color {
        r: mix(frame.r, backdrop[0]),
        g: mix(frame.g, backdrop[1]),
        b: mix(frame.b, backdrop[2]),
        a: u8::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Accent, Appearance, Contrast, wcag};

    #[test]
    fn frame_text_stays_readable_over_any_backdrop() {
        let levels = [0u8, 64, 128, 192, 255];
        for style in Style::ALL {
            for scheme in [Scheme::Dark, Scheme::Light] {
                let palette = Appearance {
                    scheme,
                    style,
                    accent: Accent::default(),
                    contrast: Contrast::default(),
                    ..Appearance::default()
                }
                .palette();
                for r in levels {
                    for g in levels {
                        for b in levels {
                            let frame =
                                over_backdrop(palette.frame, limit_backdrop([r, g, b], scheme));
                            assert!(wcag(palette.ink, frame) >= 7.0, "{style:?} {scheme:?} ink");
                            assert!(
                                wcag(palette.ink2, frame) >= 4.5,
                                "{style:?} {scheme:?} ink2"
                            );
                            assert!(
                                wcag(palette.ink3, frame) >= 3.0,
                                "{style:?} {scheme:?} ink3"
                            );
                            let panel =
                                over_backdrop(palette.panel, limit_backdrop([r, g, b], scheme));
                            assert!(
                                wcag(palette.ink, panel) >= 7.0,
                                "{style:?} {scheme:?} panel"
                            );
                            assert!(
                                wcag(palette.ink2, panel) >= 4.5,
                                "{style:?} {scheme:?} panel"
                            );
                        }
                    }
                }
            }
        }
    }
}
