use std::thread;

use bande_audio::Audio;

pub fn follow_streams(changed: impl Fn(bool) + Send + 'static) {
    thread::spawn(move || {
        loop {
            let audio = match Audio::connect() {
                Ok(audio) => audio,
                Err(error) => {
                    tracing::warn!("sound streams cannot keep the computer awake: {error}");
                    return;
                }
            };
            let mut snapshot = audio.snapshot();
            let mut streaming = snapshot.streaming;
            changed(streaming);
            while snapshot.connected {
                snapshot = audio.changed_since(Some(snapshot.generation));
                if snapshot.streaming != streaming {
                    streaming = snapshot.streaming;
                    changed(streaming);
                }
            }
            changed(false);
        }
    });
}
