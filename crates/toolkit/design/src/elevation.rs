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

pub const fn window_edge_opacity(scheme: Scheme) -> f32 {
    match scheme {
        Scheme::Light => 0.10,
        Scheme::Dark => 0.55,
    }
}

impl Elevation {
    pub fn shadows(self, scheme: Scheme, active: bool) -> [Shadow; 2] {
        let [key, ambient] = match (self, scheme) {
            (Self::Frame, Scheme::Light) => [shadow(0.5, 1.0, 0.10), shadow(6.0, 12.0, 0.14)],
            (Self::Overlay, Scheme::Light) => [shadow(0.5, 0.75, 0.10), shadow(3.0, 7.0, 0.14)],
            (Self::Frame, Scheme::Dark) => [shadow(1.0, 1.5, 0.30), shadow(8.0, 14.0, 0.42)],
            (Self::Overlay, Scheme::Dark) => [shadow(1.0, 1.0, 0.30), shadow(4.0, 8.0, 0.36)],
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
