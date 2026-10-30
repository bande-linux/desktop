#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Typeface {
    #[default]
    Interface,
    Display,
    Monospace,
}

impl Typeface {
    pub const fn family(self) -> &'static str {
        match self {
            Self::Interface => "Inter",
            Self::Display => "Manrope",
            Self::Monospace => "JetBrains Mono",
        }
    }

    pub const fn features(self, numeric: bool) -> &'static str {
        match (self, numeric) {
            (Self::Interface, false) => "\"cv05\", \"cv08\", \"ss07\", \"ss08\"",
            (Self::Interface, true) => "\"cv05\", \"cv08\", \"ss07\", \"ss08\", \"tnum\"",
            (Self::Display, false) => "",
            (Self::Display, true) => "\"tnum\"",
            (Self::Monospace, _) => "\"zero\"",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextRole {
    Display,
    Title,
    Heading,
    WindowTitle,
    Body,
    BodyStrong,
    Callout,
    Label,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypeSpec {
    pub typeface: Typeface,
    pub size: f32,
    pub line_height: f32,
    pub weight: f32,
    pub tracking: f32,
    pub caps: bool,
}

const fn spec(typeface: Typeface, size: f32, line_height: f32, weight: f32) -> TypeSpec {
    TypeSpec {
        typeface,
        size,
        line_height,
        weight,
        tracking: 0.0,
        caps: false,
    }
}

impl TextRole {
    pub const fn spec(self) -> TypeSpec {
        match self {
            Self::Display => spec(Typeface::Display, 28.0, 34.0, 700.0),
            Self::Title => spec(Typeface::Display, 22.0, 28.0, 700.0),
            Self::Heading => spec(Typeface::Display, 17.0, 22.0, 700.0),
            Self::WindowTitle => spec(Typeface::Display, 15.0, 20.0, 700.0),
            Self::Body => spec(Typeface::Interface, 13.0, 18.0, 400.0),
            Self::BodyStrong => spec(Typeface::Interface, 13.0, 18.0, 600.0),
            Self::Callout => spec(Typeface::Interface, 12.0, 16.0, 400.0),
            Self::Label => TypeSpec {
                tracking: 0.07,
                caps: true,
                ..spec(Typeface::Interface, 10.5, 14.0, 600.0)
            },
            Self::Mono => spec(Typeface::Monospace, 13.0, 18.0, 400.0),
        }
    }
}
