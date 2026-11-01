use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use rustix::process::{Pid, Signal, kill_process};

use crate::area::Region;

const LAUNCH: &str = "gst-launch-1.0";
const INSPECT: &str = "gst-inspect-1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    WebM,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::WebM => "webm",
        }
    }

    pub fn available() -> Option<Self> {
        let has = |element: &str| {
            Command::new(INSPECT)
                .arg(element)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        if !has("pipewiresrc") {
            return None;
        }
        match (
            has("x264enc") && has("mp4mux"),
            has("vp8enc") && has("webmmux"),
        ) {
            (true, _) => Some(Self::Mp4),
            (false, true) => Some(Self::WebM),
            _ => None,
        }
    }
}

fn even(value: u32) -> u32 {
    value & !1
}

pub fn pipeline(
    node: u32,
    screen: (u32, u32),
    area: Option<Region>,
    container: Container,
    path: &Path,
) -> Vec<String> {
    let mut elements = vec![
        format!("pipewiresrc path={node} always-copy=true do-timestamp=true keepalive-time=100"),
        "videoconvert".to_owned(),
    ];
    let (width, height) = match area {
        Some(region) => (region.width, region.height),
        None => screen,
    };
    let (left, top) = area.map_or((0, 0), |region| (region.x, region.y));
    let right = screen.0.saturating_sub(left + even(width));
    let bottom = screen.1.saturating_sub(top + even(height));
    if (left, top, right, bottom) != (0, 0, 0, 0) {
        elements.push(format!(
            "videocrop left={left} top={top} right={right} bottom={bottom}"
        ));
    }
    elements.push("videorate".to_owned());
    elements.push("video/x-raw,framerate=30/1".to_owned());
    match container {
        Container::Mp4 => {
            elements.push("videoconvert".to_owned());
            elements.push("video/x-raw,format=I420".to_owned());
            elements.push(
                "x264enc tune=zerolatency speed-preset=veryfast quantizer=20 pass=quant".to_owned(),
            );
            elements.push("mp4mux faststart=true".to_owned());
        }
        Container::WebM => {
            elements.push("vp8enc deadline=1 cpu-used=8 end-usage=cq cq-level=12".to_owned());
            elements.push("webmmux".to_owned());
        }
    }
    elements.push(format!("filesink location=\"{}\"", path.display()));
    let mut arguments = vec!["-e".to_owned(), "-q".to_owned()];
    for (index, element) in elements.into_iter().enumerate() {
        if index > 0 {
            arguments.push("!".to_owned());
        }
        arguments.extend(split_element(&element));
    }
    arguments
}

fn split_element(element: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in element.chars() {
        match character {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

pub struct Recording {
    child: Child,
    pub path: PathBuf,
    pub node: u32,
}

impl Recording {
    pub fn start(
        node: u32,
        screen: (u32, u32),
        area: Option<Region>,
        container: Container,
        path: PathBuf,
    ) -> io::Result<Self> {
        let child = Command::new(LAUNCH)
            .args(pipeline(node, screen, area, container, &path))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?;
        Ok(Self { child, path, node })
    }

    pub fn finish(mut self) -> io::Result<PathBuf> {
        if let Ok(id) = i32::try_from(self.child.id())
            && let Some(pid) = Pid::from_raw(id)
        {
            kill_process(pid, Signal::INT)?;
        }
        let status = self.child.wait()?;
        match status.success() && self.path.exists() {
            true => Ok(self.path),
            false => Err(io::Error::other(format!(
                "the recorder ended with {status}"
            ))),
        }
    }
}
