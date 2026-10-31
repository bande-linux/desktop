use std::path::Path;
use std::sync::Once;

use unbound_graphics::{CpuRenderer, Scene};

use crate::from_rgba;
use crate::text::{clipped_texts, overlapping_texts, untranslated_texts};

#[macro_export]
macro_rules! pseudo_problems {
    ($name:expr, $scene:expr, $data:expr) => {
        $crate::review_pseudo(env!("CARGO_TARGET_TMPDIR"), $name, $scene, $data)
    };
}

pub fn use_pseudo_locale() {
    static PSEUDO: Once = Once::new();
    PSEUDO.call_once(|| unsafe {
        std::env::set_var("LANGUAGE", "en-XA");
        std::env::set_var("LANG", "en_US.UTF-8");
    });
}

pub fn review_pseudo(scratch_dir: &str, name: &str, scene: &Scene, data: &[&str]) -> Vec<String> {
    let folder = Path::new(scratch_dir).join("pseudo");
    let frame = std::fs::create_dir_all(&folder)
        .ok()
        .and_then(|()| CpuRenderer::new().render(scene).ok());
    if let Some(frame) = frame
        && let Err(error) = from_rgba(scene.width, scene.height, frame.rgba)
            .save(folder.join(format!("{name}.png")))
    {
        panic!("{name}: {error}");
    }
    let clipped = clipped_texts(scene)
        .into_iter()
        .map(|text| format!("{name}: clipped {text:?}"));
    let overlapping = overlapping_texts(scene)
        .into_iter()
        .map(|(first, second)| format!("{name}: {first:?} overlaps {second:?}"));
    let untranslated = untranslated_texts(scene, data)
        .into_iter()
        .map(|text| format!("{name}: not translated {text:?}"));
    clipped.chain(overlapping).chain(untranslated).collect()
}
