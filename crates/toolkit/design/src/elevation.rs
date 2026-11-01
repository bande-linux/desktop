use crate::Scheme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub offset: f32,
    pub blur: f32,
    pub opacity: f32,
}

const fn shadow(offset: f32, blur: f32, opacity: f32) -> Shadow {
    Shadow {
        offset,
        blur,
        opacity,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Elevation {
    Frame,
    Overlay,
}

impl Elevation {
    pub fn shadows(self, scheme: Scheme, active: bool) -> [Shadow; 2] {
        let [key, ambient] = match (self, scheme) {
            (Self::Frame, Scheme::Light) => [shadow(1.0, 1.0, 0.10), shadow(12.0, 16.0, 0.22)],
            (Self::Overlay, Scheme::Light) => [shadow(1.0, 0.75, 0.14), shadow(6.0, 9.0, 0.18)],
            (Self::Frame, Scheme::Dark) => [shadow(1.0, 1.0, 0.40), shadow(14.0, 18.0, 0.45)],
            (Self::Overlay, Scheme::Dark) => [shadow(1.0, 0.75, 0.35), shadow(6.0, 9.0, 0.40)],
        };
        let ambient = if active {
            ambient
        } else {
            Shadow {
                opacity: ambient.opacity / 2.0,
                ..ambient
            }
        };
        [key, ambient]
    }
}
