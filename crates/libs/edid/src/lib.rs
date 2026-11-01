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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    pub make: String,
    pub model: String,
    pub serial: String,
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
