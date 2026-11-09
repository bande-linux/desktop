use std::fs::File;
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const PRIVATE_BITS: u32 = 0o077;

#[derive(Default, Clone, PartialEq, Eq)]
pub struct Config {
    pub endpoint: Option<String>,
    pub key: Option<String>,
    pub ca: Option<PathBuf>,
    pub send: bool,
    pub stack_memory: bool,
    pub signature_tag: Option<String>,
}

impl Config {
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & PRIVATE_BITS != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} can be read by others", path.display()),
            ));
        }
        let mut text = String::new();
        file.read_to_string(&mut text)?;
        Ok(Self::parse(&text))
    }

    fn parse(text: &str) -> Self {
        let mut config = Self::default();
        for line in text.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match name.trim() {
                "endpoint" => config.endpoint = Some(value.trim_end_matches('/').to_owned()),
                "key" => config.key = Some(value.to_owned()),
                "ca" => config.ca = Some(PathBuf::from(value)),
                "send" => config.send = value == "on",
                "stack_memory" => config.stack_memory = value == "on",
                "signature_tag" => config.signature_tag = Some(value.to_owned()),
                _ => {}
            }
        }
        config
    }

    pub fn has_server(&self) -> bool {
        self.endpoint.is_some()
    }

    pub fn sends_automatically(&self) -> bool {
        self.send && self.has_server()
    }
}

pub fn set_sending(path: &Path, on: bool) -> io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut lines: Vec<String> = text
        .lines()
        .filter(|line| line.split('=').next().map(str::trim) != Some("send"))
        .map(str::to_owned)
        .collect();
    lines.push(format!("send = {}", if on { "on" } else { "off" }));
    let staged = path.with_extension("new");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)?;
    std::io::Write::write_all(&mut file, format!("{}\n", lines.join("\n")).as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&staged, path)
}
