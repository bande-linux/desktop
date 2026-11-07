use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    Configure {
        nv12_dmabuf: bool,
    },
    Open(PathBuf),
    Play,
    Pause,
    Seek {
        position_ms: u64,
        exact: bool,
    },
    Step {
        forward: bool,
    },
    SetRate(f64),
    SetVolume(f64),
    SelectTracks {
        audio: Option<String>,
        subtitles: Option<String>,
    },
    QueryPosition,
    Released(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Playback {
    Opening,
    Paused,
    Playing,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureKind {
    Unreadable,
    Unsupported,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub display_width: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MediaInfo {
    pub duration_ms: Option<u64>,
    pub seekable: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub has_audio: bool,
    pub video: Option<VideoInfo>,
    pub audio_tracks: Vec<Track>,
    pub subtitle_tracks: Vec<Track>,
    pub selected_audio: Option<String>,
    pub selected_subtitles: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BufferKind {
    Shm,
    Dmabuf { modifier: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plane {
    pub offset: u32,
    pub stride: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameBuffer {
    pub id: u64,
    pub kind: BufferKind,
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub planes: Vec<Plane>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    Ready,
    Info(Box<MediaInfo>),
    State(Playback),
    Position(u64),
    Buffer(FrameBuffer),
    Frame(u64),
    Forget(u64),
    Subtitle {
        text: Option<String>,
        duration_ms: Option<u64>,
    },
    Art {
        length: u64,
    },
    Failed {
        kind: FailureKind,
        detail: String,
    },
}
