pub const KEY_ESC: u16 = 1;
pub const KEY_MINUS: u16 = 12;
pub const KEY_EQUAL: u16 = 13;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_LEFTBRACE: u16 = 26;
pub const KEY_RIGHTBRACE: u16 = 27;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_SEMICOLON: u16 = 39;
pub const KEY_APOSTROPHE: u16 = 40;
pub const KEY_GRAVE: u16 = 41;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_BACKSLASH: u16 = 43;
pub const KEY_COMMA: u16 = 51;
pub const KEY_DOT: u16 = 52;
pub const KEY_SLASH: u16 = 53;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_SPACE: u16 = 57;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_MAX: u16 = 127;

const LETTERS: [u16; 26] = [
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45,
    21, 44,
];
const DIGITS: [u16; 10] = [11, 2, 3, 4, 5, 6, 7, 8, 9, 10];
const FUNCTION_KEYS: [u16; 12] = [59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 87, 88];
const SHIFTED_DIGITS: &str = ")!@#$%^&*(";
const PUNCTUATION: [(char, char, u16); 11] = [
    ('-', '_', KEY_MINUS),
    ('=', '+', KEY_EQUAL),
    ('[', '{', KEY_LEFTBRACE),
    (']', '}', KEY_RIGHTBRACE),
    (';', ':', KEY_SEMICOLON),
    ('\'', '"', KEY_APOSTROPHE),
    ('`', '~', KEY_GRAVE),
    ('\\', '|', KEY_BACKSLASH),
    (',', '<', KEY_COMMA),
    ('.', '>', KEY_DOT),
    ('/', '?', KEY_SLASH),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Vec<u16>,
    pub key: u16,
}

fn modifier(name: &str) -> Option<u16> {
    match name {
        "super" | "meta" | "logo" => Some(KEY_LEFTMETA),
        "ctrl" | "control" => Some(KEY_LEFTCTRL),
        "alt" => Some(KEY_LEFTALT),
        "shift" => Some(KEY_LEFTSHIFT),
        _ => None,
    }
}

fn named_key(name: &str) -> Option<u16> {
    let key = match name {
        "space" => KEY_SPACE,
        "enter" | "return" => KEY_ENTER,
        "tab" => KEY_TAB,
        "escape" | "esc" => KEY_ESC,
        "backspace" => KEY_BACKSPACE,
        "delete" => 111,
        "insert" => 110,
        "home" => 102,
        "end" => 107,
        "pageup" => 104,
        "pagedown" => 109,
        "up" => 103,
        "down" => 108,
        "left" => 105,
        "right" => 106,
        "mute" => 113,
        "volumedown" => 114,
        "volumeup" => 115,
        _ => {
            if let Some(number) = name.strip_prefix('f').and_then(|n| n.parse::<usize>().ok()) {
                return number
                    .checked_sub(1)
                    .and_then(|index| FUNCTION_KEYS.get(index))
                    .copied();
            }
            let mut characters = name.chars();
            let (Some(character), None) = (characters.next(), characters.next()) else {
                return modifier(name);
            };
            return character_key(character).map(|(key, _)| key);
        }
    };
    Some(key)
}

pub fn chord(text: &str) -> Result<Chord, String> {
    let lowered = text.to_lowercase();
    let mut names: Vec<&str> = lowered.split('+').collect();
    let last = names.pop().unwrap_or_default();
    let modifiers = names
        .into_iter()
        .map(|name| modifier(name).ok_or_else(|| format!("unknown modifier {name}")))
        .collect::<Result<_, _>>()?;
    let key = named_key(last).ok_or_else(|| format!("unknown key {last}"))?;
    Ok(Chord { modifiers, key })
}

pub fn character_key(character: char) -> Option<(u16, bool)> {
    let index_of = |base: u8| usize::from(character as u8 - base);
    match character {
        'a'..='z' => Some((LETTERS[index_of(b'a')], false)),
        'A'..='Z' => Some((LETTERS[index_of(b'A')], true)),
        '0'..='9' => Some((DIGITS[index_of(b'0')], false)),
        ' ' => Some((KEY_SPACE, false)),
        '\n' => Some((KEY_ENTER, false)),
        '\t' => Some((KEY_TAB, false)),
        _ => SHIFTED_DIGITS
            .find(character)
            .map(|digit| (DIGITS[digit], true))
            .or_else(|| {
                PUNCTUATION
                    .iter()
                    .find_map(|&(plain, shifted, key)| match character {
                        c if c == plain => Some((key, false)),
                        c if c == shifted => Some((key, true)),
                        _ => None,
                    })
            }),
    }
}
