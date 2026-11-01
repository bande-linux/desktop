use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Destination {
    #[default]
    Desktop,
    Pictures,
    Clipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CaptureOptions {
    pub destination: Destination,
    pub timer: u8,
    pub show_pointer: bool,
}

pub const TIMERS: [u8; 3] = [0, 5, 10];

impl CaptureOptions {
    pub fn parse(text: &str) -> Self {
        text.lines().filter_map(|line| line.split_once('=')).fold(
            Self::default(),
            |options, (key, value)| match (key.trim(), value.trim()) {
                ("save-to", "desktop") => Self {
                    destination: Destination::Desktop,
                    ..options
                },
                ("save-to", "pictures") => Self {
                    destination: Destination::Pictures,
                    ..options
                },
                ("save-to", "clipboard") => Self {
                    destination: Destination::Clipboard,
                    ..options
                },
                ("timer", seconds) => match seconds.parse() {
                    Ok(timer) if TIMERS.contains(&timer) => Self { timer, ..options },
                    _ => options,
                },
                ("show-pointer", shown) => Self {
                    show_pointer: shown == "true",
                    ..options
                },
                _ => options,
            },
        )
    }

    pub fn serialize(&self) -> String {
        let destination = match self.destination {
            Destination::Desktop => "desktop",
            Destination::Pictures => "pictures",
            Destination::Clipboard => "clipboard",
        };
        format!(
            "save-to={destination}\ntimer={}\nshow-pointer={}\n",
            self.timer, self.show_pointer
        )
    }

    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn store(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, self.serialize())
    }
}
