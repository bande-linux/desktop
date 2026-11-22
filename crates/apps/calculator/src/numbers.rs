use bande_datetime::posix_locale;
use icu_decimal::DecimalFormatter;
use icu_decimal::input::Decimal;

const LOCALE_VARIABLES: [&str; 3] = ["LC_ALL", "LC_NUMERIC", "LANG"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Numbers {
    pub decimal: char,
    pub group: char,
}

impl Numbers {
    pub const POINT: Self = Self {
        decimal: '.',
        group: ',',
    };

    pub fn from_env() -> Self {
        let value = LOCALE_VARIABLES
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
            .unwrap_or_default();
        let locale = posix_locale(&value);
        let Ok(formatter) = DecimalFormatter::try_new((&locale).into(), Default::default()) else {
            return Self::POINT;
        };
        let mut sample = Decimal::from(12_345_678);
        sample.multiply_pow10(-1);
        let marks: Vec<char> = formatter
            .format(&sample)
            .to_string()
            .chars()
            .filter(|character| {
                !character.is_numeric() && !matches!(character, '\u{200e}' | '\u{200f}' | '\u{61c}')
            })
            .collect();
        match marks.as_slice() {
            [group, .., decimal] if group != decimal => Self {
                decimal: *decimal,
                group: *group,
            },
            _ => Self::POINT,
        }
    }

    fn is_mark(self, character: char) -> bool {
        matches!(character, '.' | ',') || character == self.decimal || character == self.group
    }

    pub fn read(self, text: &str) -> Option<String> {
        let cleaned: String = text
            .trim()
            .replace('−', "-")
            .chars()
            .filter(|character| {
                !character.is_whitespace() && !matches!(character, '_' | '\'' | '’')
            })
            .collect();
        let (number, scale) = cleaned.split_at(cleaned.find(['e', 'E']).unwrap_or(cleaned.len()));
        let marks: Vec<(usize, char)> = number
            .char_indices()
            .filter(|(_, character)| self.is_mark(*character))
            .collect();
        let Some(&(last_at, last)) = marks.last() else {
            return Some(cleaned);
        };
        let fraction_at = last_at + last.len_utf8();
        let uniform = marks.iter().all(|(_, mark)| *mark == last);
        let decimal = match (uniform, marks.len()) {
            (false, _) => true,
            (true, 1) if last == self.decimal => true,
            (true, 1) if last == self.group => false,
            (true, 1) if number[fraction_at..].chars().count() == 3 => return None,
            (true, count) => count == 1,
        };
        let (whole, point, fraction) = match decimal {
            true => (&number[..last_at], ".", &number[fraction_at..]),
            false => (number, "", ""),
        };
        let whole = whole.strip_prefix('-').unwrap_or(whole);
        let separators: Vec<char> = whole.chars().filter(|c| self.is_mark(*c)).collect();
        let mut groups = whole.split(|c| self.is_mark(c));
        let first = groups.next().unwrap_or_default();
        let grouped_well = separators.windows(2).all(|pair| pair[0] == pair[1])
            && (1..=3).contains(&first.len())
            && groups.all(|group| group.len() == 3);
        if !separators.is_empty() && !grouped_well {
            return None;
        }
        let sign = if number.starts_with('-') { "-" } else { "" };
        let digits: String = whole.chars().filter(|c| !self.is_mark(*c)).collect();
        Some(format!("{sign}{digits}{point}{fraction}{scale}"))
    }
}
