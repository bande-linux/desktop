#![no_main]

use libfuzzer_sys::fuzz_target;
use web_html::{AtomTable, TokenizerState, tokenize};

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else {
        return;
    };
    let state = match selector % 6 {
        0 => TokenizerState::Data,
        1 => TokenizerState::Plaintext,
        2 => TokenizerState::Rcdata,
        3 => TokenizerState::Rawtext,
        4 => TokenizerState::ScriptData,
        _ => TokenizerState::CdataSection,
    };
    let text = String::from_utf8_lossy(rest);
    let mut atoms = AtomTable::new();
    tokenize(&text, state, Some("title"), &mut atoms, |_, _| {});
});
