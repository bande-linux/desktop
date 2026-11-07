use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bande_content::{Kind, Pixels, decode};
use bande_media::{
    BufferKind, Command, Event, FailureKind, FrameBuffer, MediaInfo, Playback, Player,
};
use bande_toolkit::ui::SurfaceId;
use bande_toolkit::{
    Dmabuf, DmabufPlane, ExternalSurface, Image, ShmLayout, SurfaceBuffer, Window, WindowError,
};

const NV12: u32 = u32::from_le_bytes(*b"NV12");
const LINEAR: u64 = 0;
const POSITION_REFRESH: Duration = Duration::from_millis(500);
const ART_LIMIT: u64 = 32 * 1024 * 1024;

pub struct Session {
    player: Option<Player>,
    surface: Option<ExternalSurface>,
    buffers: HashMap<u64, SurfaceBuffer>,
    info: MediaInfo,
    playback: Playback,
    position_ms: u64,
    position_at: Instant,
    rate: f64,
    volume: f64,
    muted: bool,
    showing_frame: bool,
    subtitle: Option<String>,
    subtitle_until: Option<Instant>,
    art: Option<Image>,
    failure: Option<FailureKind>,
    next_refresh: Option<Instant>,
    play_when_ready: bool,
    configured: bool,
    waiting: Option<PathBuf>,
}

fn plain_text(markup: &str) -> String {
    let mut text = String::with_capacity(markup.len());
    let mut in_tag = false;
    for character in markup.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn read_art(fd: OwnedFd, length: u64) -> io::Result<Vec<u8>> {
    let mut file = File::from(fd);
    file.rewind()?;
    let mut encoded = Vec::new();
    file.take(length.min(ART_LIMIT)).read_to_end(&mut encoded)?;
    Ok(encoded)
}

fn art_image(encoded: &[u8]) -> Option<Image> {
    let kind = Kind::of(encoded, Path::new(""))?;
    let decoded = decode(encoded, kind).ok()?;
    let Pixels::Still(levels) = decoded.pixels else {
        return None;
    };
    let level = levels.into_iter().next()?;
    Image::from_premultiplied(level.width, level.height, Arc::unwrap_or_clone(level.rgba))
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            player: None,
            surface: None,
            buffers: HashMap::new(),
            info: MediaInfo::default(),
            playback: Playback::Opening,
            position_ms: 0,
            position_at: Instant::now(),
            rate: 1.0,
            volume: 1.0,
            muted: false,
            showing_frame: false,
            subtitle: None,
            subtitle_until: None,
            art: None,
            failure: None,
            next_refresh: None,
            play_when_ready: false,
            configured: false,
            waiting: None,
        }
    }

    pub fn load(&mut self, path: &Path, play: bool) -> io::Result<()> {
        if self.player.is_none() {
            self.player = Some(Player::spawn()?);
        }
        self.info = MediaInfo::default();
        self.playback = Playback::Opening;
        self.position_ms = 0;
        self.position_at = Instant::now();
        self.subtitle = None;
        self.subtitle_until = None;
        self.art = None;
        self.failure = None;
        self.next_refresh = None;
        self.showing_frame = false;
        self.play_when_ready = play;
        match self.configured {
            true => self.send(Command::Open(path.to_owned())),
            false => self.waiting = Some(path.to_owned()),
        }
        if self.rate != 1.0 {
            self.send(Command::SetRate(self.rate));
        }
        Ok(())
    }

    pub fn is_loaded(&self) -> bool {
        self.player.is_some()
    }

    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.player.as_ref().map(Player::fd)
    }

    pub fn info(&self) -> &MediaInfo {
        &self.info
    }

    pub fn playback(&self) -> Playback {
        self.playback
    }

    pub fn failure(&self) -> Option<FailureKind> {
        self.failure
    }

    pub fn art(&self) -> Option<&Image> {
        self.art.as_ref()
    }

    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    pub fn rate(&self) -> f64 {
        self.rate
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn is_playing(&self) -> bool {
        self.playback == Playback::Playing
    }

    pub fn has_video(&self) -> bool {
        self.info.video.is_some()
    }

    pub fn surface(&self) -> Option<(SurfaceId, (u32, u32))> {
        let video = self.info.video?;
        let surface = self.surface.as_ref().filter(|_| self.showing_frame)?;
        Some((surface.id(), (video.display_width, video.height)))
    }

    pub fn position(&self, now: Instant) -> Duration {
        let reported = Duration::from_millis(self.position_ms);
        let position = match self.is_playing() {
            true => reported + now.duration_since(self.position_at).mul_f64(self.rate),
            false => reported,
        };
        match self.info.duration_ms {
            Some(duration) => position.min(Duration::from_millis(duration)),
            None => position,
        }
    }

    fn send(&self, command: Command) {
        if let Some(player) = &self.player {
            player.send(command);
        }
    }

    pub fn play(&mut self) {
        if self.playback == Playback::Ended {
            self.position_ms = 0;
        }
        self.send(Command::Play);
    }

    pub fn pause(&mut self) {
        self.position_ms = self.position(Instant::now()).as_millis() as u64;
        self.position_at = Instant::now();
        self.send(Command::Pause);
    }

    pub fn toggle(&mut self) {
        match self.is_playing() {
            true => self.pause(),
            false => self.play(),
        }
    }

    pub fn seek(&mut self, position: Duration, exact: bool) {
        self.position_ms = position.as_millis() as u64;
        self.position_at = Instant::now();
        if self.playback == Playback::Ended {
            self.playback = Playback::Paused;
        }
        self.send(Command::Seek {
            position_ms: self.position_ms,
            exact,
        });
    }

    pub fn step(&mut self, forward: bool) {
        self.send(Command::Step { forward });
    }

    pub fn set_rate(&mut self, rate: f64) {
        self.position_ms = self.position(Instant::now()).as_millis() as u64;
        self.position_at = Instant::now();
        self.rate = rate;
        self.send(Command::SetRate(rate));
    }

    pub fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = false;
        self.send(Command::SetVolume(self.volume));
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.send(Command::SetVolume(match self.muted {
            true => 0.0,
            false => self.volume,
        }));
    }

    pub fn select_tracks(&self, audio: Option<String>, subtitles: Option<String>) {
        self.send(Command::SelectTracks { audio, subtitles });
    }

    pub fn deadline(&self) -> Option<Instant> {
        [self.next_refresh, self.subtitle_until]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.subtitle_until.is_some_and(|until| until <= now) {
            self.subtitle_until = None;
            self.subtitle = None;
            changed = true;
        }
        if self.next_refresh.is_some_and(|at| at <= now) {
            self.next_refresh = Some(now + POSITION_REFRESH);
            self.send(Command::QueryPosition);
        }
        changed
    }

    pub fn set_refreshing(&mut self, refreshing: bool) {
        self.next_refresh = match refreshing && self.is_playing() {
            true => self
                .next_refresh
                .or_else(|| Some(Instant::now() + POSITION_REFRESH)),
            false => None,
        };
    }

    pub fn ready(&mut self, window: &mut Window) -> bool {
        let mut changed = false;
        loop {
            let Some(player) = &self.player else {
                return changed;
            };
            match player.receive() {
                Ok(Some((event, fds))) => changed |= self.apply(window, event, fds),
                Ok(None) => return changed,
                Err(error) => {
                    tracing::warn!("the media engine stopped: {error}");
                    self.player = None;
                    self.failure = Some(FailureKind::Failed);
                    self.playback = Playback::Paused;
                    self.update_inhibit();
                    return true;
                }
            }
        }
    }

    fn apply(&mut self, window: &mut Window, event: Event, fds: Vec<OwnedFd>) -> bool {
        match event {
            Event::Ready => {
                self.send(Command::Configure {
                    nv12_dmabuf: window.supports_dmabuf(NV12, LINEAR),
                });
                self.configured = true;
                if let Some(path) = self.waiting.take() {
                    self.send(Command::Open(path));
                }
                false
            }
            Event::Info(info) => {
                self.info = *info;
                self.update_inhibit();
                true
            }
            Event::State(playback) => self.state(playback),
            Event::Position(position) => {
                self.position_ms = position;
                self.position_at = Instant::now();
                true
            }
            Event::Buffer(buffer) => {
                if let Err(error) = self.add_buffer(window, &buffer, fds) {
                    tracing::warn!("cannot show a video buffer: {error}");
                }
                false
            }
            Event::Frame(id) => self.frame(id),
            Event::Forget(id) => {
                self.buffers.remove(&id);
                false
            }
            Event::Subtitle { text, duration_ms } => {
                self.subtitle = text.map(|text| plain_text(&text));
                self.subtitle_until = duration_ms
                    .filter(|_| self.subtitle.is_some())
                    .map(|ms| Instant::now() + Duration::from_millis(ms));
                true
            }
            Event::Art { length } => {
                self.art = fds
                    .into_iter()
                    .next()
                    .and_then(|fd| read_art(fd, length).ok())
                    .and_then(|encoded| art_image(&encoded));
                true
            }
            Event::Failed { kind, detail } => {
                tracing::warn!("cannot play: {detail}");
                self.failure = Some(kind);
                true
            }
        }
    }

    fn state(&mut self, playback: Playback) -> bool {
        let opened = self.playback == Playback::Opening && playback == Playback::Paused;
        let changed = self.playback != playback;
        self.playback = playback;
        self.position_at = Instant::now();
        if opened && std::mem::take(&mut self.play_when_ready) {
            self.send(Command::Play);
        }
        if playback == Playback::Playing {
            self.send(Command::QueryPosition);
        }
        if playback == Playback::Ended
            && let Some(duration) = self.info.duration_ms
        {
            self.position_ms = duration;
        }
        self.update_inhibit();
        changed
    }

    fn update_inhibit(&mut self) {
        let inhibited = self.is_playing() && self.has_video();
        if let Some(surface) = &mut self.surface {
            surface.set_idle_inhibited(inhibited);
        }
    }

    fn add_buffer(
        &mut self,
        window: &mut Window,
        buffer: &FrameBuffer,
        fds: Vec<OwnedFd>,
    ) -> Result<(), WindowError> {
        if self.surface.is_none() {
            self.surface = Some(window.create_external_surface()?);
            self.update_inhibit();
        }
        let (Some(surface), Some(player)) = (&self.surface, &self.player) else {
            return Ok(());
        };
        let on_release = player.release_handle(buffer.id);
        let created = match buffer.kind {
            BufferKind::Shm => {
                let (Some(fd), Some(plane)) = (fds.first(), buffer.planes.first()) else {
                    return Ok(());
                };
                surface.shm_buffer(
                    fd.as_fd(),
                    ShmLayout {
                        width: buffer.width,
                        height: buffer.height,
                        stride: plane.stride,
                        offset: plane.offset,
                        fourcc: buffer.fourcc,
                    },
                    on_release,
                )?
            }
            BufferKind::Dmabuf { modifier } => {
                let planes: Vec<DmabufPlane<'_>> = buffer
                    .planes
                    .iter()
                    .zip(fds.iter().cycle())
                    .map(|(plane, fd)| DmabufPlane {
                        fd: fd.as_fd(),
                        offset: plane.offset,
                        stride: plane.stride,
                    })
                    .collect();
                surface.dmabuf_buffer(
                    &Dmabuf {
                        planes: &planes,
                        width: buffer.width,
                        height: buffer.height,
                        fourcc: buffer.fourcc,
                        modifier,
                        flipped: false,
                    },
                    on_release,
                )?
            }
        };
        self.buffers.insert(buffer.id, created);
        Ok(())
    }

    fn frame(&mut self, id: u64) -> bool {
        let (Some(surface), Some(buffer)) = (&mut self.surface, self.buffers.get(&id)) else {
            return false;
        };
        if let Err(error) = surface.present(buffer) {
            tracing::warn!("cannot present a frame: {error}");
        }
        !std::mem::replace(&mut self.showing_frame, true)
    }
}
