mod frames;
mod info;

use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use bande_media::{Channel, Command, Event, FailureKind, MediaInfo, Opening, Playback};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use rustix::fs::{MemfdFlags, memfd_create};
use rustix::net::{RecvFlags, SendFlags};

use frames::Frames;

const ONE_MILLISECOND: gst::ClockTime = gst::ClockTime::from_mseconds(1);
const COMPOSITOR_DELAY: gst::ClockTime = gst::ClockTime::from_mseconds(16);
const STEP_BACK_FALLBACK_MS: u64 = 40;
const SIDECAR_EXTENSIONS: [&str; 3] = ["srt", "vtt", "ass"];
const SUBTITLE_BYTES: u64 = 16 << 20;
const RESUME_TRIES: u32 = 40;
const RESUME_PAUSE: std::time::Duration = std::time::Duration::from_millis(50);
const RESUME_WAIT: gst::ClockTime = gst::ClockTime::from_seconds(2);
const RESUME_CLOSE: gst::ClockTime = gst::ClockTime::from_mseconds(250);

struct Status {
    info: MediaInfo,
    rate: f64,
    ended: bool,
    collection: Option<gst::StreamCollection>,
    held: bool,
    opened: u64,
    kept_audio: Option<String>,
    show_added: bool,
}

fn readable_subtitles(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= SUBTITLE_BYTES => true,
        Ok(_) => {
            eprintln!(
                "bande-media-engine: {} is not a subtitle file",
                path.display()
            );
            false
        }
        Err(_) => false,
    }
}

fn subtitle_extension(extension: &str) -> bool {
    SIDECAR_EXTENSIONS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(extension))
}

fn sidecar(video: &Path) -> Option<PathBuf> {
    let exact = SIDECAR_EXTENSIONS
        .iter()
        .map(|extension| video.with_extension(extension))
        .find(|candidate| readable_subtitles(candidate));
    if exact.is_some() {
        return exact;
    }
    let stem = video.file_stem()?.to_str()?;
    let languages = bande_l10n::preferred_languages();
    let rank = |language: &str| {
        let base = language.split(['-', '_']).next().unwrap_or(language);
        let short = gstreamer_tag::language_codes::language_code_iso_639_1(base)
            .map_or_else(|| base.to_ascii_lowercase(), |code| code.to_string());
        languages.iter().position(|wanted| *wanted == short)
    };
    let mut found: Vec<(usize, PathBuf)> = std::fs::read_dir(video.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            let (language, extension) = name
                .strip_prefix(stem)?
                .strip_prefix('.')?
                .rsplit_once('.')?;
            let wanted =
                !language.is_empty() && !language.contains('.') && subtitle_extension(extension);
            let rank = rank(language)?;
            (wanted && readable_subtitles(&path)).then_some((rank, path))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, path)| path)
}

#[derive(Clone)]
struct Engine {
    channel: Arc<Channel>,
    playbin: gst::Element,
    video: gst_app::AppSink,
    status: Arc<Mutex<Status>>,
    frames: Arc<Frames>,
}

fn emit(channel: &Channel, event: &Event) {
    if channel.send(event, &[], SendFlags::empty()).is_err() {
        std::process::exit(0);
    }
}

fn make(factory: &str) -> Result<gst::Element, gst::glib::BoolError> {
    gst::ElementFactory::make(factory).build()
}

impl Engine {
    fn new(channel: Arc<Channel>) -> Result<Self, gst::glib::BoolError> {
        let frames = Arc::new(Frames::new(Arc::clone(&channel)));
        let video = frames::video_sink(Arc::clone(&frames), COMPOSITOR_DELAY);
        let subtitles = frames::subtitle_sink(Arc::clone(&channel));
        let playbin = make("playbin3")?;
        playbin.set_property("video-sink", &video);
        playbin.set_property("text-sink", &subtitles);
        if let Ok(pitch) = make("scaletempo") {
            playbin.set_property("audio-filter", &pitch);
        }
        Ok(Self {
            channel,
            playbin,
            video,
            status: Arc::new(Mutex::new(Status {
                info: MediaInfo::default(),
                rate: 1.0,
                ended: false,
                collection: None,
                held: false,
                opened: 0,
                kept_audio: None,
                show_added: false,
            })),
            frames,
        })
    }

    fn failed(&self, kind: FailureKind, detail: String) {
        emit(&self.channel, &Event::Failed { kind, detail });
    }

    fn with_info(&self, change: impl FnOnce(&mut MediaInfo)) {
        let Ok(mut status) = self.status.lock() else {
            return;
        };
        let before = status.info.clone();
        change(&mut status.info);
        if status.info != before {
            emit(&self.channel, &Event::Info(Box::new(status.info.clone())));
        }
    }

    fn position(&self) -> Option<gst::ClockTime> {
        self.playbin.query_position::<gst::ClockTime>()
    }

    fn seek_to(&self, position: gst::ClockTime, flags: gst::SeekFlags, rate: f64) {
        let sought = self.playbin.seek(
            rate,
            flags,
            gst::SeekType::Set,
            position,
            gst::SeekType::None,
            gst::ClockTime::NONE,
        );
        if let Err(error) = sought {
            eprintln!("bande-media-engine: cannot seek: {error}");
        }
    }

    fn rate(&self) -> f64 {
        self.status.lock().map_or(1.0, |status| status.rate)
    }

    fn open(&self, opening: Opening) {
        let Ok(uri) = gst::glib::filename_to_uri(&opening.path, None) else {
            return self.failed(FailureKind::Unreadable, opening.path.display().to_string());
        };
        let added = opening
            .subtitles
            .filter(|subtitles| readable_subtitles(subtitles));
        self.set_state(gst::State::Ready);
        if let Ok(mut status) = self.status.lock() {
            status.show_added = added.is_some();
            status.kept_audio = added.as_ref().and(status.info.selected_audio.clone());
            status.info = MediaInfo::default();
            status.ended = false;
            status.collection = None;
            status.held = opening.start_ms > 0;
            status.opened += 1;
        }
        self.frames.new_media();
        self.playbin.set_property("uri", uri.as_str());
        let subtitles = added
            .or_else(|| sidecar(&opening.path))
            .and_then(|subtitles| gst::glib::filename_to_uri(subtitles, None).ok());
        self.playbin
            .set_property("suburi", subtitles.as_ref().map(|uri| uri.as_str()));
        emit(&self.channel, &Event::State(Playback::Opening));
        if self.playbin.set_state(gst::State::Paused).is_err() {
            eprintln!("bande-media-engine: the pipeline refused to start");
        }
        if opening.start_ms > 0 {
            self.resume_in_background(ONE_MILLISECOND * opening.start_ms);
        }
    }

    fn play(&self) {
        let ended = self
            .status
            .lock()
            .is_ok_and(|mut status| std::mem::take(&mut status.ended));
        if ended {
            self.seek_to(gst::ClockTime::ZERO, gst::SeekFlags::FLUSH, self.rate());
        }
        if self.playbin.set_state(gst::State::Playing).is_err() {
            eprintln!("bande-media-engine: cannot play");
        }
    }

    fn set_state(&self, state: gst::State) {
        if let Err(error) = self.playbin.set_state(state) {
            eprintln!("bande-media-engine: cannot go to {state:?}: {error}");
        }
    }

    fn step(&self, forward: bool) {
        self.set_state(gst::State::Paused);
        if forward {
            let step = gst::event::Step::new(gst::format::Buffers::ONE, 1.0, true, false);
            self.video.send_event(step);
            return;
        }
        let back = self
            .frames
            .frame_duration()
            .unwrap_or(gst::ClockTime::from_mseconds(STEP_BACK_FALLBACK_MS));
        let target = self.position().unwrap_or_default().saturating_sub(back);
        self.seek_to(
            target,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            self.rate(),
        );
    }

    fn select_tracks(&self, audio: Option<String>, subtitles: Option<String>) {
        let Ok(status) = self.status.lock() else {
            return;
        };
        let Some(collection) = status.collection.as_ref() else {
            return;
        };
        let ids = info::chosen_streams(collection, audio.as_deref(), subtitles.as_deref());
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        self.playbin
            .send_event(gst::event::SelectStreams::new(ids.iter().copied()));
    }

    fn command(&self, command: Command) {
        match command {
            Command::Configure { nv12_dmabuf } => self.frames.configure(&self.video, nv12_dmabuf),
            Command::Open(opening) => self.open(opening),
            Command::Play => self.play(),
            Command::Pause => {
                self.set_state(gst::State::Paused);
            }
            Command::Seek { position_ms, exact } => {
                if let Ok(mut status) = self.status.lock() {
                    status.ended = false;
                }
                let accuracy = match exact {
                    true => gst::SeekFlags::ACCURATE,
                    false => gst::SeekFlags::KEY_UNIT | gst::SeekFlags::SNAP_NEAREST,
                };
                self.seek_to(
                    ONE_MILLISECOND * position_ms,
                    gst::SeekFlags::FLUSH | accuracy,
                    self.rate(),
                );
            }
            Command::Step { forward } => self.step(forward),
            Command::SetRate(rate) => {
                if let Ok(mut status) = self.status.lock() {
                    status.rate = rate;
                }
                self.seek_to(
                    self.position().unwrap_or_default(),
                    gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                    rate,
                );
            }
            Command::SetVolume(volume) => self.playbin.set_property("volume", volume),
            Command::SelectTracks { audio, subtitles } => self.select_tracks(audio, subtitles),
            Command::QueryPosition => {
                if let Some(position) = self.position() {
                    emit(&self.channel, &Event::Position(position.mseconds()));
                }
            }
            Command::Released(id) => self.frames.released(id),
        }
    }

    fn refresh_extent(&self) {
        let duration = self
            .playbin
            .query_duration::<gst::ClockTime>()
            .map(gst::ClockTime::mseconds);
        let mut query = gst::query::Seeking::new(gst::Format::Time);
        let seekable = self.playbin.query(&mut query) && query.result().0;
        self.with_info(|info| {
            info.duration_ms = duration.or(info.duration_ms);
            info.seekable = seekable;
        });
    }

    fn send_art(&self, encoded: &[u8]) {
        let sent = memfd_create("bande-media-art", MemfdFlags::CLOEXEC)
            .map_err(std::io::Error::from)
            .and_then(|fd: OwnedFd| {
                std::fs::File::from(fd.try_clone()?).write_all(encoded)?;
                let event = Event::Art {
                    length: encoded.len() as u64,
                };
                self.channel.send(&event, &[fd.as_fd()], SendFlags::empty())
            });
        if let Err(error) = sent {
            eprintln!("bande-media-engine: cannot pass the cover art: {error}");
        }
    }

    fn state_changed(&self, new: gst::State) {
        let waiting = self
            .status
            .lock()
            .map_or(true, |status| status.ended || status.held);
        match new {
            gst::State::Playing => emit(&self.channel, &Event::State(Playback::Playing)),
            gst::State::Paused if !waiting => self.paused(),
            _ => {}
        }
    }

    fn paused(&self) {
        self.refresh_extent();
        emit(&self.channel, &Event::State(Playback::Paused));
    }

    fn resume_in_background(&self, start: gst::ClockTime) {
        let opened = self.status.lock().map_or(0, |status| status.opened);
        let engine = self.clone();
        let spawned = std::thread::Builder::new()
            .name("resume".to_owned())
            .spawn(move || engine.resume_at(start, opened));
        if let Err(error) = spawned {
            eprintln!("bande-media-engine: cannot resume where playback stopped: {error}");
            if let Ok(mut status) = self.status.lock() {
                status.held = false;
            }
        }
    }

    fn resume_at(&self, start: gst::ClockTime, opened: u64) {
        let current = || {
            self.status
                .lock()
                .is_ok_and(|status| status.opened == opened)
        };
        for _ in 0..RESUME_TRIES {
            std::thread::sleep(RESUME_PAUSE);
            if !current() {
                return;
            }
            let (_, state, pending) = self.playbin.state(RESUME_WAIT);
            if state != gst::State::Paused || pending != gst::State::VoidPending {
                continue;
            }
            let sought = self.playbin.seek(
                self.rate(),
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                start,
                gst::SeekType::None,
                gst::ClockTime::NONE,
            );
            if sought.is_err() {
                continue;
            }
            if let (Err(error), ..) = self.playbin.state(RESUME_WAIT) {
                eprintln!("bande-media-engine: cannot resume where playback stopped: {error}");
                continue;
            }
            let near = self
                .position()
                .is_some_and(|position| position.max(start) - position.min(start) < RESUME_CLOSE);
            if near {
                break;
            }
        }
        let held = self
            .status
            .lock()
            .is_ok_and(|mut status| status.opened == opened && std::mem::take(&mut status.held));
        if held {
            self.paused();
        }
    }

    fn handle(&self, message: &gst::Message) {
        use gst::MessageView;
        let from_playbin = message.src() == Some(self.playbin.upcast_ref());
        match message.view() {
            MessageView::Eos(_) => {
                if let Ok(mut status) = self.status.lock() {
                    status.ended = true;
                }
                self.set_state(gst::State::Paused);
                emit(&self.channel, &Event::State(Playback::Ended));
            }
            MessageView::Error(error) => {
                let (kind, detail) = info::failure(error.error(), error.debug().as_deref());
                self.failed(kind, detail);
            }
            MessageView::Element(element) => {
                if let Some(detail) = info::missing_plugin(element.structure()) {
                    self.failed(FailureKind::Unsupported, detail);
                }
            }
            MessageView::StateChanged(changed) if from_playbin => {
                self.state_changed(changed.current());
            }
            MessageView::AsyncDone(_) | MessageView::DurationChanged(_) => self.refresh_extent(),
            MessageView::Tag(tag) => {
                let tags = tag.tags();
                self.with_info(|media| info::apply_tags(media, &tags));
                if let Some(art) = info::cover_art(&tags) {
                    self.send_art(&art);
                }
            }
            MessageView::StreamCollection(message) => {
                let collection = message.stream_collection();
                let (audio, subtitles) = match self.status.lock() {
                    Ok(mut status) => {
                        status.collection = Some(collection.clone());
                        let added = status
                            .show_added
                            .then(|| {
                                collection
                                    .iter()
                                    .filter(|stream| {
                                        stream.stream_type().contains(gst::StreamType::TEXT)
                                    })
                                    .filter_map(|stream| stream.stream_id())
                                    .next_back()
                                    .map(|id| id.to_string())
                            })
                            .flatten();
                        (status.kept_audio.clone(), added)
                    }
                    Err(_) => (None, None),
                };
                self.with_info(|media| info::apply_collection(media, &collection));
                self.select_tracks(audio, subtitles);
            }
            MessageView::StreamsSelected(message) => {
                let selected: Vec<gst::Stream> = message.streams().collect();
                self.with_info(|media| info::apply_selection(media, &selected));
            }
            _ => {}
        }
    }

    fn follow_bus(&self) -> Result<(), std::io::Error> {
        let Some(bus) = self.playbin.bus() else {
            return Err(std::io::ErrorKind::NotFound.into());
        };
        let engine = self.clone();
        std::thread::Builder::new()
            .name("bus".to_owned())
            .spawn(move || {
                for message in bus.iter_timed(gst::ClockTime::NONE) {
                    engine.handle(&message);
                }
            })?;
        Ok(())
    }

    fn set_video(&self, video: bande_media::VideoInfo) {
        self.with_info(|media| media.video = Some(video));
    }
}

fn run(channel: Arc<Channel>) -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: called before GStreamer starts any thread, so no other thread reads the locale.
    unsafe { libc::setlocale(libc::LC_MESSAGES, c"".as_ptr()) };
    gst::init()?;
    let engine = Engine::new(Arc::clone(&channel))?;
    let reporter = engine.clone();
    engine
        .frames
        .on_video(Box::new(move |video| reporter.set_video(video)));
    engine.follow_bus()?;
    emit(&channel, &Event::Ready);
    while let Ok(Some((command, _))) = channel.receive::<Command>(RecvFlags::empty()) {
        engine.command(command);
    }
    Ok(())
}

fn main() -> ExitCode {
    let channel = match std::io::stdin().as_fd().try_clone_to_owned() {
        Ok(socket) => Arc::new(Channel::from_fd(socket)),
        Err(error) => {
            eprintln!("bande-media-engine: no socket: {error}");
            return ExitCode::FAILURE;
        }
    };
    match run(channel) {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("bande-media-engine: {error}");
            ExitCode::FAILURE
        }
    }
}
