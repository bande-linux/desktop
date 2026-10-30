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

#[cfg(test)]
mod tests {
    use super::*;

    fn block(vendor: &str, product: u16, number: u32) -> Vec<u8> {
        let mut block = vec![0u8; 128];
        block[..8].copy_from_slice(&HEADER);
        let letters: Vec<u16> = vendor
            .bytes()
            .map(|byte| u16::from(byte - b'A' + 1))
            .collect();
        let packed = (letters[0] << 10) | (letters[1] << 5) | letters[2];
        block[8..10].copy_from_slice(&packed.to_be_bytes());
        block[10..12].copy_from_slice(&product.to_le_bytes());
        block[12..16].copy_from_slice(&number.to_le_bytes());
        block
    }

    fn describe(block: &mut [u8], at: usize, kind: u8, text: &str) {
        block[at..at + 5].copy_from_slice(&[0, 0, 0, kind, 0]);
        let mut padded = text.as_bytes().to_vec();
        padded.push(b'\n');
        padded.resize(13, b' ');
        block[at + 5..at + 18].copy_from_slice(&padded);
    }

    #[test]
    fn a_monitor_is_named_by_its_maker_name_and_serial_descriptors() {
        let mut edid = block("DEL", 0xa0ec, 12345);
        describe(&mut edid, 72, MONITOR_NAME, "DELL U2720Q");
        describe(&mut edid, 90, SERIAL_TEXT, "7ZD5K53");

        assert_eq!(
            parse(&edid),
            Some(Edid {
                make: "Dell".to_owned(),
                model: "DELL U2720Q".to_owned(),
                serial: "7ZD5K53".to_owned(),
            })
        );
    }

    #[test]
    fn a_panel_without_descriptors_falls_back_to_its_codes() {
        let edid = block("BOE", 0x0a2d, 0);

        assert_eq!(
            parse(&edid),
            Some(Edid {
                make: "BOE".to_owned(),
                model: "BOE 0A2D".to_owned(),
                serial: String::new(),
            })
        );
        let unknown = block("XYZ", 1, 42);
        assert_eq!(
            parse(&unknown).map(|edid| (edid.make, edid.serial)),
            Some(("XYZ".to_owned(), "42".to_owned()))
        );
    }

    #[test]
    fn identical_monitors_are_told_apart_by_their_connector() {
        let displays = [
            ("DP-1", identity("Dell", "DELL P2419H", "")),
            ("DP-2", identity("Dell", "DELL P2419H", "")),
            ("eDP-1", identity("", "eDP-1", "")),
        ];

        assert_eq!(
            distinct_identities(&displays),
            [
                "Dell DELL P2419H (DP-1)",
                "Dell DELL P2419H (DP-2)",
                "eDP-1"
            ]
        );
    }

    #[test]
    fn anything_but_an_edid_block_is_refused() {
        assert_eq!(parse(&[0u8; 128]), None);
        assert_eq!(parse(&HEADER), None);
    }
}
