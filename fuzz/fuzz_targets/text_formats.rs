#![no_main]

use bande_content::{html, markdown, rtf};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = rtf(text);
    let _ = html(text);
    let _ = markdown(text);
});
