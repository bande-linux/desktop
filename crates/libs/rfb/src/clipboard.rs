use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

use crate::protocol::{CutText, MOST_CUT_TEXT};

const TEXT: u32 = 1;
const CAPS: u32 = 1 << 24;
const REQUEST: u32 = 1 << 25;
const PEEK: u32 = 1 << 26;
const NOTIFY: u32 = 1 << 27;
const PROVIDE: u32 = 1 << 28;
const FORMATS: u32 = 0xffff;

/// Clipboard text in both directions, over the extended clipboard pseudo-encoding when the
/// other side has it (UTF-8, compressed, sent when asked for) and as Latin-1 otherwise.
#[derive(Default)]
pub struct Clipboard {
    extended: Option<u32>,
    offered: Option<String>,
    announced: bool,
}

/// What a clipboard message means for this side.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub reply: Option<CutText>,
    pub copied: Option<String>,
}

fn message(flags: u32, data: &[u8]) -> CutText {
    let mut bytes = flags.to_be_bytes().to_vec();
    bytes.extend_from_slice(data);
    CutText::Extended(bytes)
}

fn be32(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?))
}

fn compressed_text(text: &str) -> Option<Vec<u8>> {
    let mut text = text
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .into_bytes();
    text.push(0);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&(text.len() as u32).to_be_bytes())
        .and_then(|()| encoder.write_all(&text))
        .ok()?;
    encoder.finish().ok()
}

fn provided_text(data: &[u8]) -> Option<String> {
    let mut inflated = Vec::new();
    ZlibDecoder::new(data)
        .take(MOST_CUT_TEXT as u64 + 4)
        .read_to_end(&mut inflated)
        .ok()?;
    let length = be32(&inflated)? as usize;
    let text = inflated.get(4..4 + length)?;
    let text = text.strip_suffix(&[0]).unwrap_or(text);
    String::from_utf8(text.to_vec())
        .ok()
        .map(|text| text.replace("\r\n", "\n"))
}

impl Clipboard {
    /// What this side can do, sent once: by a server when the viewer asks for the extended
    /// clipboard, by a viewer in answer to the server's.
    pub fn announce(&mut self) -> CutText {
        self.announced = true;
        message(
            CAPS | REQUEST | NOTIFY | PROVIDE | TEXT,
            &(MOST_CUT_TEXT as u32).to_be_bytes(),
        )
    }

    /// The message that tells the other side about `text`, copied here.
    pub fn copied_here(&mut self, text: String) -> CutText {
        match self.extended {
            Some(actions) if actions & NOTIFY != 0 => {
                self.offered = Some(text);
                message(NOTIFY | TEXT, &[])
            }
            _ => CutText::Latin1(text),
        }
    }

    /// Takes a clipboard message from the other side.
    pub fn received(&mut self, text: CutText) -> Outcome {
        let bytes = match text {
            CutText::Latin1(text) => {
                return Outcome {
                    reply: None,
                    copied: Some(text),
                };
            }
            CutText::Extended(bytes) => bytes,
        };
        let Some(flags) = be32(&bytes) else {
            return Outcome::default();
        };
        let data = &bytes[4..];
        let mut outcome = Outcome::default();
        if flags & CAPS != 0 {
            self.extended = Some(flags & !FORMATS);
            if !self.announced {
                outcome.reply = Some(self.announce());
            }
        } else if flags & PROVIDE != 0 && flags & TEXT != 0 {
            outcome.copied = provided_text(data);
        } else if flags & (REQUEST | PEEK) != 0 && flags & TEXT != 0 {
            outcome.reply = match (flags & REQUEST != 0, &self.offered) {
                (true, Some(text)) => {
                    compressed_text(text).map(|data| message(PROVIDE | TEXT, &data))
                }
                _ => Some(message(
                    NOTIFY | if self.offered.is_some() { TEXT } else { 0 },
                    &[],
                )),
            };
        } else if flags & NOTIFY != 0 && flags & TEXT != 0 {
            outcome.reply = Some(message(REQUEST | TEXT, &[]));
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_text_crosses_the_extended_clipboard() {
        let (mut here, mut there) = (Clipboard::default(), Clipboard::default());
        let offer = here.announce();
        let answer = there.received(offer).reply.unwrap();
        assert_eq!(here.received(answer).reply, None);
        let notify = here.copied_here("grüße\nκόσμε".to_owned());
        let request = there.received(notify).reply.unwrap();
        let provide = here.received(request).reply.unwrap();
        assert_eq!(
            there.received(provide).copied.as_deref(),
            Some("grüße\nκόσμε")
        );
        let mut plain = Clipboard::default();
        assert_eq!(
            plain.copied_here("é".to_owned()),
            CutText::Latin1("é".to_owned())
        );
    }
}
