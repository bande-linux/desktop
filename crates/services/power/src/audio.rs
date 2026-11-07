use std::thread;

use bande_audio::Audio;

pub fn follow_streams(changed: impl Fn(bool) + Send + 'static) {
    thread::spawn(move || {
        let audio = match Audio::start() {
            Ok(audio) => audio,
            Err(error) => {
                tracing::warn!("sound streams cannot keep the computer awake: {error}");
                return;
            }
        };
        let mut generation = None;
        let mut streaming = None;
        loop {
            let snapshot = audio.changed_since(generation);
            generation = Some(snapshot.generation);
            if streaming != Some(snapshot.streaming) {
                streaming = Some(snapshot.streaming);
                changed(snapshot.streaming);
            }
        }
    });
}
