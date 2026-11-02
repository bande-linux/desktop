use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;

use bande_audio::{Audio, Level, Snapshot};
use bande_inbox::Outbox;

use crate::Update;
use crate::devices::{SoundControl, report};
use crate::status::{Output, Sound};

fn sound_of(snapshot: &Snapshot) -> Option<Sound> {
    let output = snapshot.default_output()?;
    Some(Sound {
        volume: output.level.volume,
        muted: output.level.muted,
        outputs: snapshot
            .outputs
            .iter()
            .map(|device| Output {
                name: device.name.clone(),
                description: device.description.clone(),
            })
            .collect(),
        default_output: snapshot.default_output.clone(),
    })
}

fn control_sound(audio: &Audio, control: SoundControl) -> Result<(), bande_audio::Error> {
    let snapshot = audio.snapshot();
    match control {
        SoundControl::UseOutput(name) => match snapshot.device(&name) {
            Some(device) => audio.set_default(device),
            None => Ok(()),
        },
        SoundControl::SetMuted(muted) => match snapshot.default_output() {
            Some(output) => audio.set_muted(output, muted),
            None => Ok(()),
        },
        SoundControl::Nudge(delta) => match snapshot.default_output() {
            Some(output) => audio.set_level(
                output,
                Level {
                    volume: (output.level.volume + delta).clamp(0.0, 1.0),
                    muted: false,
                },
            ),
            None => Ok(()),
        },
    }
}

fn run_sound(audio: &Audio, outbox: &Outbox<Update>) {
    let mut generation = None;
    let mut capturing = None;
    loop {
        let snapshot = audio.changed_since(generation);
        generation = Some(snapshot.generation);
        if !outbox.send(Update::Sound(sound_of(&snapshot))) {
            return;
        }
        if capturing.as_ref() != Some(&snapshot.capturing) {
            capturing = Some(snapshot.capturing.clone());
            if !outbox.send(Update::Capturing(snapshot.capturing)) {
                return;
            }
        }
    }
}

pub(crate) fn follow_sound(audio: Arc<Audio>, outbox: Outbox<Update>) -> Sender<SoundControl> {
    let (controls, received) = mpsc::channel::<SoundControl>();
    let controlled = Arc::clone(&audio);
    thread::spawn(move || {
        for control in received {
            report("change the sound", control_sound(&controlled, control));
        }
    });
    thread::spawn(move || run_sound(&audio, &outbox));
    controls
}
