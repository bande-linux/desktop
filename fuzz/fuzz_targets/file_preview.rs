#![no_main]

use bande_content::{Kind, decode};
use libfuzzer_sys::fuzz_target;

const KINDS: [Kind; 6] = [
    Kind::Png,
    Kind::Jpeg,
    Kind::Gif,
    Kind::Webp,
    Kind::Svg,
    Kind::Pdf,
];

fuzz_target!(|data: &[u8]| {
    if let Some((choice, bytes)) = data.split_first() {
        let _ = decode(bytes, KINDS[usize::from(*choice) % KINDS.len()]);
    }
});
