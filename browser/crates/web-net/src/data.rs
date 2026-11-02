use url::Url;

use crate::fetch::MAX_BODY;
use crate::{ContentType, FetchError, Response};

const DEFAULT_TYPE: &[u8] = b"text/plain;charset=US-ASCII";

fn hex(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

fn percent_decoded(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        let escaped = (byte == b'%')
            .then(|| Some((hex(*bytes.get(index + 1)?)? << 4) | hex(*bytes.get(index + 2)?)?))
            .flatten();
        match escaped {
            Some(value) => {
                decoded.push(value);
                index += 3;
            }
            None => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    decoded
}

fn base64(bytes: &[u8]) -> Option<Vec<u8>> {
    let value = |byte: u8| match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let mut decoded = Vec::with_capacity(bytes.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0);
    for &byte in bytes.iter().filter(|byte| !byte.is_ascii_whitespace()) {
        if byte == b'=' {
            break;
        }
        buffer = (buffer << 6) | u32::from(value(byte)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            decoded.push((buffer >> bits) as u8);
        }
    }
    Some(decoded)
}

pub(crate) fn fetch(url: Url) -> Result<Response, FetchError> {
    let text = &url[url::Position::BeforePath..];
    let (header, data) = text.split_once(',').ok_or(FetchError::InvalidUrl)?;
    let header = percent_decoded(header.trim());
    let (header, encoded) = match header.strip_suffix(b";base64") {
        Some(rest) => (rest.to_vec(), true),
        None => (header, false),
    };
    let raw = percent_decoded(data);
    if raw.len() > MAX_BODY {
        return Err(FetchError::TooLarge);
    }
    let body = match encoded {
        true => base64(&raw).ok_or(FetchError::BadEncoding)?,
        false => raw,
    };
    let content_type = match header.is_empty() {
        true => ContentType::parse(DEFAULT_TYPE),
        false => ContentType::parse(&header).or_else(|| ContentType::parse(DEFAULT_TYPE)),
    };
    Ok(Response {
        url,
        status: 200,
        content_type,
        body,
    })
}
