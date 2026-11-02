use std::borrow::Cow;

pub use encoding_rs::Encoding;
use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE};

fn charset_label(bytes: &[u8]) -> Option<&[u8]> {
    let rest = bytes.strip_prefix(b"@charset \"")?;
    let end = rest.iter().position(|&b| b == b'"')?;
    let (label, tail) = rest.split_at(end);
    (tail.starts_with(b"\";") && label.iter().all(|b| b.is_ascii() && *b != b'"')).then_some(label)
}

pub fn decode_stylesheet<'a>(
    bytes: &'a [u8],
    protocol_label: Option<&str>,
    environment: Option<&'static Encoding>,
) -> (Cow<'a, str>, &'static Encoding) {
    let fallback = protocol_label
        .and_then(|label| Encoding::for_label(label.trim().as_bytes()))
        .or_else(|| {
            charset_label(bytes)
                .and_then(Encoding::for_label)
                .map(|encoding| {
                    if encoding == UTF_16BE || encoding == UTF_16LE {
                        UTF_8
                    } else {
                        encoding
                    }
                })
        })
        .or(environment)
        .unwrap_or(UTF_8);
    let (text, encoding, _) = fallback.decode(bytes);
    (text, encoding)
}
