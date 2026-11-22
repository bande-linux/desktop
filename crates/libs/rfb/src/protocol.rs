pub const VERSION: &[u8; 12] = b"RFB 003.008\n";
pub const SECURITY_VNC: u8 = 2;
pub const SECURITY_VENCRYPT: u8 = 19;
pub const VENCRYPT_X509_VNC: u32 = 261;
pub const VENCRYPT_X509_PLAIN: u32 = 262;
pub const ENCODING_RAW: i32 = 0;
pub const ENCODING_ZRLE: i32 = 16;
pub const ENCODING_DESKTOP_SIZE: i32 = -223;
pub const ENCODING_DESKTOP_NAME: i32 = -307;
pub const ENCODING_EXTENDED_CLIPBOARD: i32 = 0xc0a1_e5ce_u32 as i32;
pub const MOST_CUT_TEXT: usize = 1 << 20;
pub const LARGEST_SCREEN: u16 = 8192;
const MOST_ZRLE: usize = 64 << 20;
const MOST_NAME: usize = 4096;
const MOST_ENCODINGS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V3_3,
    V3_7,
    V3_8,
}

pub fn parse_version(line: &[u8; 12]) -> Option<Version> {
    let text = std::str::from_utf8(line).ok()?;
    let numbers = text.strip_prefix("RFB ")?.strip_suffix('\n')?;
    let (major, minor) = numbers.split_once('.')?;
    let all_digits = |part: &str| part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit());
    if !all_digits(major) || !all_digits(minor) || major != "003" {
        return None;
    }
    match minor.parse::<u32>().ok()? {
        0..=6 => Some(Version::V3_3),
        7 => Some(Version::V3_7),
        _ => Some(Version::V3_8),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth: u8,
    pub big_endian: bool,
    pub true_colour: bool,
    pub red_max: u16,
    pub green_max: u16,
    pub blue_max: u16,
    pub red_shift: u8,
    pub green_shift: u8,
    pub blue_shift: u8,
}

impl PixelFormat {
    pub const NATIVE: Self = Self {
        bits_per_pixel: 32,
        depth: 24,
        big_endian: false,
        true_colour: true,
        red_max: 255,
        green_max: 255,
        blue_max: 255,
        red_shift: 16,
        green_shift: 8,
        blue_shift: 0,
    };

    pub fn parse(bytes: &[u8; 16]) -> Self {
        let max = |at: usize| u16::from_be_bytes([bytes[at], bytes[at + 1]]);
        Self {
            bits_per_pixel: bytes[0],
            depth: bytes[1],
            big_endian: bytes[2] != 0,
            true_colour: bytes[3] != 0,
            red_max: max(4),
            green_max: max(6),
            blue_max: max(8),
            red_shift: bytes[10],
            green_shift: bytes[11],
            blue_shift: bytes[12],
        }
    }

    pub fn to_bytes(self) -> [u8; 16] {
        let [red_high, red_low] = self.red_max.to_be_bytes();
        let [green_high, green_low] = self.green_max.to_be_bytes();
        let [blue_high, blue_low] = self.blue_max.to_be_bytes();
        [
            self.bits_per_pixel,
            self.depth,
            u8::from(self.big_endian),
            u8::from(self.true_colour),
            red_high,
            red_low,
            green_high,
            green_low,
            blue_high,
            blue_low,
            self.red_shift,
            self.green_shift,
            self.blue_shift,
            0,
            0,
            0,
        ]
    }

    pub fn is_usable(&self) -> bool {
        let fits = |max: u16, shift: u8| {
            u32::from(shift) < u32::from(self.bits_per_pixel)
                && (u64::from(max) << shift) < (1u64 << self.bits_per_pixel)
        };
        self.true_colour
            && matches!(self.bits_per_pixel, 8 | 16 | 32)
            && fits(self.red_max, self.red_shift)
            && fits(self.green_max, self.green_shift)
            && fits(self.blue_max, self.blue_shift)
    }

    pub fn bytes_per_pixel(&self) -> usize {
        usize::from(self.bits_per_pixel / 8)
    }

    pub fn value(&self, rgb: u32) -> u32 {
        let scale = |channel: u32, max: u16| (channel * u32::from(max) + 127) / 255;
        let red = scale((rgb >> 16) & 0xff, self.red_max);
        let green = scale((rgb >> 8) & 0xff, self.green_max);
        let blue = scale(rgb & 0xff, self.blue_max);
        (red << self.red_shift) | (green << self.green_shift) | (blue << self.blue_shift)
    }

    pub fn put(&self, value: u32, out: &mut Vec<u8>) {
        let bytes = self.bytes_per_pixel();
        match self.big_endian {
            true => out.extend_from_slice(&value.to_be_bytes()[4 - bytes..]),
            false => out.extend_from_slice(&value.to_le_bytes()[..bytes]),
        }
    }

    pub fn compact(&self) -> Option<Compact> {
        if self.bits_per_pixel != 32 || self.depth > 24 {
            return None;
        }
        let used = (u32::from(self.red_max) << self.red_shift)
            | (u32::from(self.green_max) << self.green_shift)
            | (u32::from(self.blue_max) << self.blue_shift);
        match (used & 0xff00_0000 == 0, used & 0xff == 0) {
            (true, _) => Some(Compact::Low),
            (false, true) => Some(Compact::High),
            (false, false) => None,
        }
    }

    pub fn put_compact(&self, value: u32, compact: Option<Compact>, out: &mut Vec<u8>) {
        match (compact, self.big_endian) {
            (None, _) => self.put(value, out),
            (Some(Compact::Low), false) => out.extend_from_slice(&value.to_le_bytes()[..3]),
            (Some(Compact::Low), true) => out.extend_from_slice(&value.to_be_bytes()[1..]),
            (Some(Compact::High), false) => out.extend_from_slice(&value.to_le_bytes()[1..]),
            (Some(Compact::High), true) => out.extend_from_slice(&value.to_be_bytes()[..3]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compact {
    Low,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMessage {
    SetPixelFormat(PixelFormat),
    SetEncodings(Vec<i32>),
    UpdateRequest { incremental: bool, area: Area },
    Key { down: bool, keysym: u32 },
    Pointer { buttons: u8, x: u16, y: u16 },
    CutText(CutText),
}

/// Clipboard text: Latin-1 as RFC 6143 has it, or a message of the extended clipboard
/// pseudo-encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutText {
    Latin1(String),
    Extended(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    UnknownMessage(u8),
    TooLong,
}

fn cut_text_length(message: &[u8]) -> Result<usize, Malformed> {
    let length = be32(message, 4) as i32;
    let length = usize::try_from(length.unsigned_abs()).unwrap_or(usize::MAX);
    match length > MOST_CUT_TEXT {
        true => Err(Malformed::TooLong),
        false => Ok(length),
    }
}

fn cut_text(message: &[u8]) -> CutText {
    match (be32(message, 4) as i32) < 0 {
        true => CutText::Extended(message[8..].to_vec()),
        false => CutText::Latin1(message[8..].iter().map(|&byte| char::from(byte)).collect()),
    }
}

/// A cut text message of `kind` (3 from a server, 6 from a viewer).
pub fn cut_text_message(kind: u8, text: &CutText) -> Vec<u8> {
    let (length, bytes): (i32, Vec<u8>) = match text {
        CutText::Latin1(text) => {
            let bytes: Vec<u8> = text
                .chars()
                .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
                .collect();
            (bytes.len() as i32, bytes)
        }
        CutText::Extended(bytes) => (-(bytes.len() as i32), bytes.clone()),
    };
    let mut message = vec![kind, 0, 0, 0];
    message.extend_from_slice(&length.to_be_bytes());
    message.extend_from_slice(&bytes);
    message
}

fn be16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub fn parse_message(buffer: &[u8]) -> Result<Option<(ClientMessage, usize)>, Malformed> {
    let Some(&kind) = buffer.first() else {
        return Ok(None);
    };
    let needed = match kind {
        0 => 20,
        2 => match buffer.len() >= 4 {
            true => {
                let count = usize::from(be16(buffer, 2));
                if count > MOST_ENCODINGS {
                    return Err(Malformed::TooLong);
                }
                4 + count * 4
            }
            false => return Ok(None),
        },
        3 => 10,
        4 => 8,
        5 => 6,
        6 => match buffer.len() >= 8 {
            true => 8 + cut_text_length(buffer)?,
            false => return Ok(None),
        },
        other => return Err(Malformed::UnknownMessage(other)),
    };
    let Some(message) = buffer.get(..needed) else {
        return Ok(None);
    };
    let parsed = match kind {
        0 => {
            let mut format = [0; 16];
            format.copy_from_slice(&message[4..20]);
            ClientMessage::SetPixelFormat(PixelFormat::parse(&format))
        }
        2 => ClientMessage::SetEncodings(
            message[4..]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|bytes| i32::from_be_bytes(*bytes))
                .collect(),
        ),
        3 => ClientMessage::UpdateRequest {
            incremental: message[1] != 0,
            area: Area {
                x: be16(message, 2),
                y: be16(message, 4),
                width: be16(message, 6),
                height: be16(message, 8),
            },
        },
        4 => ClientMessage::Key {
            down: message[1] != 0,
            keysym: be32(message, 4),
        },
        5 => ClientMessage::Pointer {
            buttons: message[1],
            x: be16(message, 2),
            y: be16(message, 4),
        },
        _ => ClientMessage::CutText(cut_text(message)),
    };
    Ok(Some((parsed, needed)))
}

pub fn server_init(width: u16, height: u16, name: &str) -> Vec<u8> {
    let name = name.as_bytes();
    let mut init = Vec::with_capacity(24 + name.len());
    init.extend_from_slice(&width.to_be_bytes());
    init.extend_from_slice(&height.to_be_bytes());
    init.extend_from_slice(&PixelFormat::NATIVE.to_bytes());
    init.extend_from_slice(&(name.len() as u32).to_be_bytes());
    init.extend_from_slice(name);
    init
}

pub fn rectangle_header(area: Area, encoding: i32, out: &mut Vec<u8>) {
    out.extend_from_slice(&area.x.to_be_bytes());
    out.extend_from_slice(&area.y.to_be_bytes());
    out.extend_from_slice(&area.width.to_be_bytes());
    out.extend_from_slice(&area.height.to_be_bytes());
    out.extend_from_slice(&encoding.to_be_bytes());
}

/// A framebuffer update that only renames the screen (the DesktopName pseudo-encoding).
pub fn desktop_name(name: &str) -> Vec<u8> {
    let mut message = Vec::new();
    update_header(1, &mut message);
    rectangle_header(
        Area {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        },
        ENCODING_DESKTOP_NAME,
        &mut message,
    );
    message.extend_from_slice(&(name.len() as u32).to_be_bytes());
    message.extend_from_slice(name.as_bytes());
    message
}

pub fn update_header(rectangles: u16, out: &mut Vec<u8>) {
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&rectangles.to_be_bytes());
}

/// A rectangle of a framebuffer update, its pixel data not yet decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rectangle {
    pub area: Area,
    pub encoding: i32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMessage {
    Update(Vec<Rectangle>),
    Bell,
    CutText(CutText),
    Colours,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    UnknownMessage(u8),
    UnknownEncoding(i32),
    OutsideScreen,
    TooLong,
}

fn parse_update(
    buffer: &[u8],
    mut size: (u16, u16),
) -> Result<Option<(ServerMessage, usize)>, Refused> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let count = be16(buffer, 2);
    let mut at = 4;
    let mut found = Vec::with_capacity(usize::from(count).min(1024));
    for _ in 0..count {
        let Some(header) = buffer.get(at..at + 12) else {
            return Ok(None);
        };
        let area = Area {
            x: be16(header, 0),
            y: be16(header, 2),
            width: be16(header, 4),
            height: be16(header, 6),
        };
        let encoding = be32(header, 8) as i32;
        at += 12;
        let length = match encoding {
            ENCODING_DESKTOP_SIZE => {
                if area.width > LARGEST_SCREEN || area.height > LARGEST_SCREEN {
                    return Err(Refused::OutsideScreen);
                }
                size = (area.width, area.height);
                0
            }
            ENCODING_RAW | ENCODING_ZRLE => {
                let right = u32::from(area.x) + u32::from(area.width);
                let bottom = u32::from(area.y) + u32::from(area.height);
                if right > u32::from(size.0) || bottom > u32::from(size.1) {
                    return Err(Refused::OutsideScreen);
                }
                match encoding {
                    ENCODING_RAW => usize::from(area.width) * usize::from(area.height) * 4,
                    _ => {
                        let Some(length) = buffer.get(at..at + 4) else {
                            return Ok(None);
                        };
                        at += 4;
                        let length = be32(length, 0) as usize;
                        if length > MOST_ZRLE {
                            return Err(Refused::TooLong);
                        }
                        length
                    }
                }
            }
            ENCODING_DESKTOP_NAME => {
                let Some(length) = buffer.get(at..at + 4) else {
                    return Ok(None);
                };
                at += 4;
                let length = be32(length, 0) as usize;
                if length > MOST_NAME {
                    return Err(Refused::TooLong);
                }
                length
            }
            other => return Err(Refused::UnknownEncoding(other)),
        };
        if buffer.len() < at + length {
            return Ok(None);
        }
        found.push((area, encoding, at..at + length));
        at += length;
    }
    let rectangles = found
        .into_iter()
        .map(|(area, encoding, data)| Rectangle {
            area,
            encoding,
            data: buffer[data].to_vec(),
        })
        .collect();
    Ok(Some((ServerMessage::Update(rectangles), at)))
}

/// The next whole message from a server, for a viewer of a screen of `size` that asked for
/// 32-bit pixels, Raw, ZRLE and DesktopSize.
pub fn parse_server_message(
    buffer: &[u8],
    size: (u16, u16),
) -> Result<Option<(ServerMessage, usize)>, Refused> {
    let Some(&kind) = buffer.first() else {
        return Ok(None);
    };
    match kind {
        0 => parse_update(buffer, size),
        1 => {
            if buffer.len() < 6 {
                return Ok(None);
            }
            let length = 6 + usize::from(be16(buffer, 4)) * 6;
            Ok((buffer.len() >= length).then_some((ServerMessage::Colours, length)))
        }
        2 => Ok(Some((ServerMessage::Bell, 1))),
        3 => {
            if buffer.len() < 8 {
                return Ok(None);
            }
            let length = 8 + cut_text_length(buffer).map_err(|_| Refused::TooLong)?;
            Ok(buffer
                .get(..length)
                .map(|message| (ServerMessage::CutText(cut_text(message)), length)))
        }
        other => Err(Refused::UnknownMessage(other)),
    }
}

pub fn set_pixel_format(format: PixelFormat) -> Vec<u8> {
    let mut message = vec![0, 0, 0, 0];
    message.extend_from_slice(&format.to_bytes());
    message
}

pub fn set_encodings(encodings: &[i32]) -> Vec<u8> {
    let mut message = vec![2, 0];
    message.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
    for encoding in encodings {
        message.extend_from_slice(&encoding.to_be_bytes());
    }
    message
}

pub fn update_request(incremental: bool, area: Area) -> Vec<u8> {
    let mut message = vec![3, u8::from(incremental)];
    for value in [area.x, area.y, area.width, area.height] {
        message.extend_from_slice(&value.to_be_bytes());
    }
    message
}

pub fn key_event(keysym: u32, down: bool) -> Vec<u8> {
    let mut message = vec![4, u8::from(down), 0, 0];
    message.extend_from_slice(&keysym.to_be_bytes());
    message
}

pub fn pointer_event(buttons: u8, x: u16, y: u16) -> Vec<u8> {
    let mut message = vec![5, buttons];
    message.extend_from_slice(&x.to_be_bytes());
    message.extend_from_slice(&y.to_be_bytes());
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_parsed_only_when_complete_and_bounded() {
        let pointer = [5, 1, 0, 10, 0, 20];
        assert_eq!(parse_message(&pointer[..5]), Ok(None));
        assert_eq!(
            parse_message(&pointer),
            Ok(Some((
                ClientMessage::Pointer {
                    buttons: 1,
                    x: 10,
                    y: 20
                },
                6
            )))
        );
        let huge_text = [6, 0, 0, 0, 0x7f, 0xff, 0xff, 0xff];
        assert_eq!(parse_message(&huge_text), Err(Malformed::TooLong));
        let many_encodings = [2, 0, 0xff, 0xff];
        assert_eq!(parse_message(&many_encodings), Err(Malformed::TooLong));
        assert_eq!(parse_message(&[200]), Err(Malformed::UnknownMessage(200)));
        assert_eq!(parse_version(b"RFB 003.889\n"), Some(Version::V3_8));
        assert_eq!(parse_version(b"RFB 003.003\n"), Some(Version::V3_3));
        assert_eq!(parse_version(b"RFB 004.000\n"), None);
        let odd = PixelFormat {
            red_shift: 30,
            ..PixelFormat::NATIVE
        };
        assert!(!odd.is_usable());
    }
}
