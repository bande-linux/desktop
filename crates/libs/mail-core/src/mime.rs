use std::hash::{BuildHasher, RandomState};

use data_encoding::BASE64;

pub const LINE_LENGTH: usize = 76;
const WORD_BYTES: usize = 45;

pub fn encoded_words(text: &str) -> String {
    if text.is_ascii() && !text.contains("=?") {
        return text.to_owned();
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for character in text.chars() {
        if chunk.len() + character.len_utf8() > WORD_BYTES {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(character);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words
        .iter()
        .map(|word| format!("=?UTF-8?B?{}?=", BASE64.encode(word.as_bytes())))
        .collect::<Vec<_>>()
        .join("\r\n ")
}

pub fn quoted_printable(text: &str) -> String {
    let mut encoded = String::new();
    let text = text.replace("\r\n", "\n");
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            encoded.push_str("\r\n");
        }
        let mut width = 0;
        let bytes = line.as_bytes();
        for (position, byte) in bytes.iter().enumerate() {
            let last = position + 1 == bytes.len();
            let literal = match byte {
                b'=' => false,
                b' ' | b'\t' => !last,
                33..=126 => !(width == 0 && *byte == b'.'),
                _ => false,
            };
            let piece = match literal {
                true => (*byte as char).to_string(),
                false => format!("={byte:02X}"),
            };
            if width + piece.len() > LINE_LENGTH - 1 {
                encoded.push_str("=\r\n");
                width = 0;
            }
            width += piece.len();
            encoded.push_str(&piece);
        }
    }
    encoded
}

pub fn unique_token() -> String {
    let state = RandomState::new();
    let now = jiff::Timestamp::now().as_nanosecond();
    format!(
        "{:016x}{:08x}",
        state.hash_one((now, std::process::id())),
        now as u32
    )
}
