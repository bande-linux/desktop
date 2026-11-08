const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];
const DESCRIPTORS: [usize; 4] = [54, 72, 90, 108];
const MONITOR_NAME: u8 = 0xfc;
const SERIAL_TEXT: u8 = 0xff;

const VENDORS: [(&str, &str); 24] = [
    ("ACR", "Acer"),
    ("AOC", "AOC"),
    ("APP", "Apple"),
    ("AUO", "AU Optronics"),
    ("BNQ", "BenQ"),
    ("BOE", "BOE"),
    ("CMN", "Chimei Innolux"),
    ("DEL", "Dell"),
    ("ENC", "Eizo"),
    ("GSM", "LG"),
    ("HPN", "HP"),
    ("HWP", "HP"),
    ("IVM", "Iiyama"),
    ("LEN", "Lenovo"),
    ("LGD", "LG Display"),
    ("MSI", "MSI"),
    ("NEC", "NEC"),
    ("PHL", "Philips"),
    ("SAM", "Samsung"),
    ("SDC", "Samsung Display"),
    ("SHP", "Sharp"),
    ("SNY", "Sony"),
    ("VSC", "ViewSonic"),
    ("XMI", "Xiaomi"),
];

const SRGB_DEFAULT: u8 = 1 << 2;
const UNDEFINED_GAMMA: u8 = 0xff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Primaries {
    pub red: (u16, u16),
    pub green: (u16, u16),
    pub blue: (u16, u16),
    pub white: (u16, u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    pub make: String,
    pub model: String,
    pub serial: String,
    pub primaries: Primaries,
    pub default_is_srgb: bool,
    pub gamma: Option<u16>,
}

fn primaries(block: &[u8]) -> Primaries {
    let coordinate = |high: usize, low: u8, shift: u8| {
        (u16::from(block[high]) << 2) | u16::from((low >> shift) & 3)
    };
    let (first, second) = (block[25], block[26]);
    Primaries {
        red: (coordinate(27, first, 6), coordinate(28, first, 4)),
        green: (coordinate(29, first, 2), coordinate(30, first, 0)),
        blue: (coordinate(31, second, 6), coordinate(32, second, 4)),
        white: (coordinate(33, second, 2), coordinate(34, second, 0)),
    }
}

fn descriptor_text(block: &[u8], kind: u8) -> Option<String> {
    DESCRIPTORS.iter().find_map(|&at| {
        let descriptor = block.get(at..at + 18)?;
        if descriptor[..3] != [0, 0, 0] || descriptor[3] != kind {
            return None;
        }
        let text: String = descriptor[5..]
            .iter()
            .take_while(|byte| **byte != b'\n')
            .map(|byte| char::from(*byte))
            .collect();
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    })
}

pub fn parse(block: &[u8]) -> Option<Edid> {
    if block.len() < 128 || block[..8] != HEADER {
        return None;
    }
    let vendor = u16::from_be_bytes([block[8], block[9]]);
    let letter = |shift: u16| char::from(b'A' - 1 + ((vendor >> shift) & 0x1f) as u8);
    let code: String = [letter(10), letter(5), letter(0)].into_iter().collect();
    let make = VENDORS
        .iter()
        .find(|(known, _)| *known == code)
        .map_or(code.clone(), |(_, name)| (*name).to_owned());
    let product = u16::from_le_bytes([block[10], block[11]]);
    let number = u32::from_le_bytes([block[12], block[13], block[14], block[15]]);
    let model =
        descriptor_text(block, MONITOR_NAME).unwrap_or_else(|| format!("{code} {product:04X}"));
    let serial = descriptor_text(block, SERIAL_TEXT)
        .or_else(|| (number != 0).then(|| number.to_string()))
        .unwrap_or_default();
    Some(Edid {
        make,
        model,
        serial,
        primaries: primaries(block),
        default_is_srgb: block[24] & SRGB_DEFAULT != 0,
        gamma: (block[23] != UNDEFINED_GAMMA).then(|| u16::from(block[23]) + 100),
    })
}

pub fn identity(make: &str, model: &str, serial: &str) -> String {
    [make, model, serial]
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn display_name(make: &str, model: &str) -> Option<String> {
    let make = make.trim();
    let model = model.trim();
    match (make.is_empty(), model.is_empty()) {
        (_, true) => None,
        (true, false) => Some(model.to_owned()),
        (false, false) if model.to_lowercase().contains(&make.to_lowercase()) => {
            Some(model.to_owned())
        }
        (false, false) => Some(format!("{make} {model}")),
    }
}

pub fn distinct_identities(displays: &[(&str, String)]) -> Vec<String> {
    displays
        .iter()
        .map(|(connector, identity)| {
            let shared = displays
                .iter()
                .filter(|(_, other)| other == identity)
                .count();
            if shared > 1 {
                format!("{identity} ({connector})")
            } else {
                identity.clone()
            }
        })
        .collect()
}
