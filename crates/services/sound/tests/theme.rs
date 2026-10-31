#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use bande_config::AlertSound;
use bande_sound::{Alert, Policy, RATE, Sound, theme_files};

fn installed(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../data/sounds/Bande")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn pcm(wav: &[u8]) -> Vec<i16> {
    wav[44..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| i16::from_le_bytes(*bytes))
        .collect()
}

#[test]
fn the_shipped_theme_is_what_the_synthesiser_makes() {
    for (name, made) in theme_files() {
        let shipped = installed(&name);
        if !name.ends_with(".wav") {
            assert_eq!(
                shipped, made,
                "{name}: run bande-sound --write-theme data/sounds/Bande"
            );
            continue;
        }
        assert_eq!(shipped[..44], made[..44], "{name} header");
        let differs = pcm(&shipped)
            .iter()
            .zip(pcm(&made))
            .any(|(shipped, made)| shipped.abs_diff(made) > 1);
        assert!(
            !differs,
            "{name} is stale: run bande-sound --write-theme data/sounds/Bande"
        );
    }
}

#[test]
fn the_theme_names_every_freedesktop_event_the_desktop_makes() {
    let names: Vec<String> = theme_files().into_iter().map(|(name, _)| name).collect();
    for event in [
        "bell",
        "message-new-instant",
        "screen-capture",
        "trash-empty",
        "device-added",
        "device-removed",
        "battery-low",
        "audio-volume-change",
    ] {
        assert!(
            names.contains(&format!("stereo/{event}.wav")),
            "{event} in {names:?}"
        );
    }
}

#[test]
fn every_sound_is_short_soft_and_ends_in_silence() {
    let alerts = Alert::ALL.map(Sound::Alert);
    for sound in Sound::THEME.into_iter().chain(alerts) {
        let samples = sound.samples();
        let peak = samples
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(
            samples.len() <= RATE as usize / 4,
            "{sound:?} lasts too long"
        );
        assert!(peak > 0.05 && peak <= 0.5, "{sound:?} peaks at {peak}");
        assert!(
            samples.last().is_some_and(|last| last.abs() < 1e-3),
            "{sound:?} is cut off"
        );
    }
    assert!(Sound::VolumeChange.samples().len() <= RATE as usize / 25);
}

#[test]
fn freedesktop_names_and_their_aliases_find_their_sound_and_the_chosen_alert() {
    assert_eq!(
        Sound::from_name("bell", Alert::Knock),
        Some(Sound::Alert(Alert::Knock))
    );
    assert_eq!(
        Sound::from_name("dialog-warning", Alert::Tick),
        Some(Sound::Alert(Alert::Tick))
    );
    assert_eq!(
        Sound::from_name("camera-shutter", Alert::Mallet),
        Some(Sound::ScreenCapture)
    );
    assert_eq!(
        Sound::from_name("power-plug", Alert::Mallet),
        Some(Sound::DeviceAdded)
    );
    assert_eq!(
        Sound::from_name("alert-low", Alert::Mallet),
        Some(Sound::Alert(Alert::Low))
    );
    assert_eq!(Sound::from_name("alert-gong", Alert::Mallet), None);
    assert_eq!(Sound::from_name("window-close", Alert::Mallet), None);
}

#[test]
fn the_switches_silence_only_what_they_name_and_the_alert_volume_scales_all() {
    let quiet = Policy {
        effects: false,
        volume_feedback: false,
        alert_volume: 0.5,
        ..Policy::default()
    };
    assert_eq!(quiet.gain(Sound::Alert(Alert::Mallet)), Some(0.5));
    assert_eq!(quiet.gain(Sound::Message), Some(0.5));
    assert_eq!(quiet.gain(Sound::ScreenCapture), None);
    assert_eq!(quiet.gain(Sound::TrashEmpty), None);
    assert_eq!(quiet.gain(Sound::VolumeChange), None);
    assert_eq!(Policy::default().gain(Sound::VolumeChange), Some(0.75));

    let silent = Policy {
        alert_volume: 0.0,
        ..Policy::default()
    };
    assert_eq!(silent.gain(Sound::Alert(Alert::Mallet)), None);
}

#[test]
fn every_alert_sound_the_configuration_accepts_has_a_sound() {
    for alert in AlertSound::ALL {
        assert!(
            Alert::from_keyword(alert.keyword()).is_some(),
            "{}",
            alert.keyword()
        );
    }
    assert_eq!(Alert::ALL.len(), AlertSound::ALL.len());
}
