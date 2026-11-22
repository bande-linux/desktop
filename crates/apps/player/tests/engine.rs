#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::process::{Child, Command as Process, Stdio};
use std::time::{Duration, Instant};

use bande_media::{Channel, Command, Event, FailureKind, Opening, Playback};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::net::{RecvFlags, SendFlags};

const PATIENCE: Duration = Duration::from_secs(30);
const RATE: u32 = 8000;

struct Engine {
    channel: Channel,
    process: Child,
}

impl Engine {
    fn start() -> Self {
        let (ours, theirs) = Channel::pair().unwrap();
        let process = Process::new(env!("CARGO_BIN_EXE_bande-media-engine"))
            .env("PULSE_SERVER", "unix:/nonexistent")
            .env("LC_ALL", "pt_PT.UTF-8")
            .env_remove("LANGUAGE")
            .stdin(Stdio::from(theirs.into_fd()))
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            channel: ours,
            process,
        }
    }

    fn send(&self, command: Command) {
        self.channel
            .send(&command, &[], SendFlags::empty())
            .unwrap();
    }

    fn next(&self, within: Duration) -> Option<Event> {
        let timeout = Timespec::try_from(within).unwrap();
        let mut fds = [PollFd::new(&self.channel, PollFlags::IN)];
        poll(&mut fds, Some(&timeout)).unwrap();
        match self.channel.receive::<Event>(RecvFlags::DONTWAIT) {
            Ok(received) => received.map(|(event, _)| event),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => None,
            Err(error) => panic!("the engine stopped: {error}"),
        }
    }

    fn until(&self, mut wanted: impl FnMut(&Event) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the engine did not answer in time");
            if self.next(left).is_some_and(|event| wanted(&event)) {
                return;
            }
        }
    }

    fn position_is(&self, wanted_ms: u64) -> bool {
        (0..50).any(|_| {
            self.send(Command::QueryPosition);
            std::iter::from_fn(|| self.next(Duration::from_millis(100)))
                .any(|event| matches!(event, Event::Position(ms) if ms.abs_diff(wanted_ms) < 100))
        })
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

fn write_wav(path: &Path, seconds: u32) {
    let samples = RATE * seconds;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + samples * 2).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(samples * 2).to_le_bytes());
    wav.resize(wav.len() + (samples * 2) as usize, 0);
    std::fs::write(path, wav).unwrap();
}

#[test]
fn a_song_opens_with_its_length_and_plays_to_the_end() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("song.wav");
    write_wav(&path, 1);
    let engine = Engine::start();
    engine.send(Command::Open(Opening {
        path,
        ..Opening::default()
    }));
    engine.until(|event| {
        matches!(event, Event::Info(info) if info.duration_ms.is_some_and(|ms| ms.abs_diff(1000) < 100))
    });
    engine.send(Command::Play);
    engine.until(|event| *event == Event::State(Playback::Ended));
}

#[test]
fn a_file_that_is_not_media_fails_without_killing_the_engine() {
    let folder = tempfile::tempdir().unwrap();
    let junk = folder.path().join("junk.mp4");
    std::fs::write(&junk, vec![0x5a; 4096]).unwrap();
    let engine = Engine::start();
    engine.send(Command::Open(Opening {
        path: junk,
        ..Opening::default()
    }));
    engine.until(
        |event| matches!(event, Event::Failed { kind, .. } if *kind == FailureKind::Unsupported),
    );
    let song = folder.path().join("song.wav");
    write_wav(&song, 1);
    engine.send(Command::Open(Opening {
        path: song,
        ..Opening::default()
    }));
    engine.until(|event| *event == Event::State(Playback::Paused));
}

#[test]
fn subtitles_named_after_a_recording_open_with_it() {
    let folder = tempfile::tempdir().unwrap();
    let song = folder.path().join("song.wav");
    write_wav(&song, 1);
    std::fs::write(
        folder.path().join("song.pt-BR.srt"),
        "1\n00:00:00,000 --> 00:00:00,900\nHello\n",
    )
    .unwrap();
    let engine = Engine::start();
    engine.send(Command::Open(Opening {
        path: song.clone(),
        ..Opening::default()
    }));
    engine.until(|event| matches!(event, Event::Info(info) if !info.subtitle_tracks.is_empty()));
    let added = folder.path().join("added.vtt");
    std::fs::write(&added, "WEBVTT\n\n00:00.000 --> 00:00.900\nHej\n").unwrap();
    engine.send(Command::Open(Opening {
        path: song,
        subtitles: Some(added),
        start_ms: 500,
    }));
    let (mut shown, mut paused) = (false, false);
    engine.until(|event| {
        shown |= matches!(event, Event::Info(info)
            if info.selected_subtitles.is_some() && info.selected_audio.is_some());
        paused |= *event == Event::State(Playback::Paused);
        shown && paused
    });
    assert!(
        engine.position_is(500),
        "playback did not carry on where it was"
    );
}
