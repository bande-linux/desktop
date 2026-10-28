#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Self = Self::hex(0).with_alpha(0);

    pub const fn hex(rgb: u32) -> Self {
        let [_, r, g, b] = rgb.to_be_bytes();
        Self {
            r,
            g,
            b,
            a: u8::MAX,
        }
    }

    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::TRANSPARENT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub surface_background: Color,
    pub surface_elevated: Color,
    pub label_primary: Color,
    pub label_muted: Color,
    pub accent_color: Color,
    pub separator_color: Color,
}

impl Palette {
    pub const fn for_scheme(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Light => Self {
                surface_background: Color::hex(0xF4F5F7),
                surface_elevated: Color::hex(0xFFFFFF),
                label_primary: Color::hex(0x1A1C20),
                label_muted: Color::hex(0x5F646D),
                accent_color: Color::hex(0x2F6FEB),
                separator_color: Color::hex(0xD5D8DE),
            },
            Scheme::Dark => Self {
                surface_background: Color::hex(0x17191D),
                surface_elevated: Color::hex(0x23262C),
                label_primary: Color::hex(0xECEEF2),
                label_muted: Color::hex(0x9EA3AC),
                accent_color: Color::hex(0x5B8DEF),
                separator_color: Color::hex(0x363A42),
            },
        }
    }
}

pub struct Spacing;

impl Spacing {
    pub const XXS: u32 = 2;
    pub const XS: u32 = 4;
    pub const S: u32 = 8;
    pub const M: u32 = 12;
    pub const L: u32 = 16;
    pub const XL: u32 = 24;
    pub const XXL: u32 = 32;
}
