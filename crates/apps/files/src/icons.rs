use std::collections::HashMap;
use std::path::PathBuf;

use bande_toolkit::{Color, Image};
use bande_xdg::{ICON_THEMES, find_icon};

pub struct Icons {
    data_dirs: Vec<PathBuf>,
    loaded: HashMap<(String, u32), Option<Image>>,
    tinted: HashMap<(String, u32, [u8; 4]), Option<Image>>,
}

impl Icons {
    pub fn new(data_dirs: Vec<PathBuf>) -> Self {
        Self {
            data_dirs,
            loaded: HashMap::new(),
            tinted: HashMap::new(),
        }
    }

    fn load(&mut self, name: &str, pixels: u32) -> Option<Image> {
        let data_dirs = &self.data_dirs;
        self.loaded
            .entry((name.to_owned(), pixels))
            .or_insert_with(|| {
                find_icon(name, pixels, &ICON_THEMES, data_dirs)
                    .and_then(|path| Image::load(&path, pixels))
            })
            .clone()
    }

    pub fn first(&mut self, names: &[&str], pixels: u32) -> Option<Image> {
        names.iter().find_map(|name| self.load(name, pixels))
    }

    pub fn symbol(&mut self, names: &[&str], pixels: u32, color: Color) -> Option<Image> {
        let name = names.first()?;
        let key = (
            (*name).to_owned(),
            pixels,
            [color.r, color.g, color.b, color.a],
        );
        if let Some(image) = self.tinted.get(&key) {
            return image.clone();
        }
        let image = self
            .first(names, pixels)
            .and_then(|image| image.tinted(color));
        self.tinted.insert(key, image.clone());
        image
    }
}
