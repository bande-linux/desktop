const LONGEST_WORD: usize = 64;

pub fn stands_alone(character: char) -> bool {
    matches!(character,
        '\u{3040}'..='\u{30ff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7af}'
        | '\u{f900}'..='\u{faff}'
        | '\u{20000}'..='\u{2fa1f}')
}

fn spelled_out(character: char) -> Option<&'static str> {
    Some(match character {
        'æ' | 'Æ' => "ae",
        'ð' | 'Ð' | 'đ' | 'Đ' => "d",
        'ı' => "i",
        'ł' | 'Ł' => "l",
        'ø' | 'Ø' => "o",
        'œ' | 'Œ' => "oe",
        'ß' | 'ẞ' => "ss",
        'þ' | 'Þ' => "th",
        _ => return None,
    })
}

fn folded(word: &str) -> String {
    let mut folded = String::with_capacity(word.len());
    for character in word.chars() {
        match spelled_out(character) {
            Some(letters) => folded.push_str(letters),
            None => folded.push(character),
        }
    }
    folded
}

pub fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(char::is_alphanumeric) {
        rest = rest.get(start..).unwrap_or_default();
        let mut characters = rest.char_indices();
        let end = match characters.next() {
            Some((_, first)) if stands_alone(first) => first.len_utf8(),
            _ => characters
                .find(|(_, character)| !character.is_alphanumeric() || stands_alone(*character))
                .map_or(rest.len(), |(index, _)| index),
        };
        let word = rest.get(..end).unwrap_or_default();
        if word.len() < LONGEST_WORD {
            words.push(folded(word));
        }
        rest = rest.get(end..).unwrap_or_default();
    }
    words
}

pub fn indexed(text: &str) -> String {
    words(text).join(" ")
}
