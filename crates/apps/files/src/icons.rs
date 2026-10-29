use std::collections::HashMap;
use std::path::PathBuf;

use unbound_toolkit::Image;
use unbound_xdg::find_icon;

const THEMES: [&str; 1] = ["Adwaita"];

pub struct Icons {
    data_dirs: Vec<PathBuf>,
    loaded: HashMap<(String, u32), Option<Image>>,
}

impl Icons {
    pub fn new(data_dirs: Vec<PathBuf>) -> Self {
        Self {
            data_dirs,
            loaded: HashMap::new(),
        }
    }

    fn load(&mut self, name: &str, pixels: u32) -> Option<Image> {
        let data_dirs = &self.data_dirs;
        self.loaded
            .entry((name.to_owned(), pixels))
            .or_insert_with(|| {
                find_icon(name, pixels, &THEMES, data_dirs)
                    .and_then(|path| Image::load(&path, pixels))
            })
            .clone()
    }

    pub fn first(&mut self, names: &[&str], pixels: u32) -> Option<Image> {
        names.iter().find_map(|name| self.load(name, pixels))
    }
}
