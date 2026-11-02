const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn is_delimiter(character: char) -> bool {
    matches!(character, '\t' | ' '..='/' | ';'..='@' | '['..='`' | '{'..='~')
}

fn digits(token: &str, min: usize, max: usize) -> Option<u32> {
    let count = token.chars().take_while(char::is_ascii_digit).count();
    if count < min || count > max {
        return None;
    }
    token.get(..count)?.parse().ok()
}

fn time(token: &str) -> Option<(u32, u32, u32)> {
    let mut parts = token.splitn(3, ':');
    let hour = digits(parts.next()?, 1, 2)?;
    let minute = digits(parts.next()?, 1, 2)?;
    let second = digits(parts.next()?, 1, 2)?;
    Some((hour, minute, second))
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(crate) fn parse_cookie_date(text: &str) -> Option<u64> {
    let (mut clock, mut day, mut month, mut year) = (None, None, None, None);
    for token in text.split(is_delimiter).filter(|token| !token.is_empty()) {
        if clock.is_none()
            && let Some(found) = time(token)
        {
            clock = Some(found);
        } else if day.is_none()
            && let Some(found) = digits(token, 1, 2)
        {
            day = Some(found);
        } else if month.is_none()
            && let Some(index) = MONTHS.iter().position(|name| {
                token
                    .get(..3)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(name))
            })
        {
            month = Some(index as u32 + 1);
        } else if year.is_none()
            && let Some(found) = digits(token, 2, 4)
        {
            year = Some(found);
        }
    }
    let (hour, minute, second) = clock?;
    let (day, month, mut year) = (day?, month?, year?);
    if (70..=99).contains(&year) {
        year += 1900;
    } else if year <= 69 {
        year += 2000;
    }
    if !(1..=31).contains(&day) || year < 1601 || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let days = days_from_civil(i64::from(year), month, day);
    let seconds = days * 86_400 + i64::from(hour * 3600 + minute * 60 + second);
    u64::try_from(seconds).ok()
}
