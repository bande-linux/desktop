use crate::parser::{ParseError, ParseResult, Parser};
use crate::tokenizer::Token;

fn digits_value(text: &str) -> ParseResult<i32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ParseError::Invalid);
    }
    Ok(text
        .parse::<i64>()
        .map_or(i32::MAX, |v| v.min(i32::MAX as i64) as i32))
}

fn parse_n_rest(input: &mut Parser<'_, '_>, a: i32, rest: &str) -> ParseResult<(i32, i32)> {
    let rest = rest.to_ascii_lowercase();
    match rest.as_str() {
        "n" => parse_b(input, a),
        "n-" => Ok((a, -parse_signless(input)?)),
        _ => match rest.strip_prefix("n-") {
            Some(digits) => Ok((a, digits_value(digits)?.saturating_neg())),
            None => Err(ParseError::Invalid),
        },
    }
}

fn parse_signless(input: &mut Parser<'_, '_>) -> ParseResult<i32> {
    match input.next_token()? {
        Token::Number(n) if n.is_integer && !n.has_sign => n.int_value().ok_or(ParseError::Invalid),
        _ => Err(ParseError::Invalid),
    }
}

fn parse_b(input: &mut Parser<'_, '_>, a: i32) -> ParseResult<(i32, i32)> {
    let state = input.state();
    match input.next_token() {
        Ok(Token::Delim('+')) => Ok((a, parse_signless(input)?)),
        Ok(Token::Delim('-')) => Ok((a, parse_signless(input)?.saturating_neg())),
        Ok(Token::Number(n)) if n.is_integer && n.has_sign => {
            Ok((a, n.int_value().ok_or(ParseError::Invalid)?))
        }
        _ => {
            input.reset(&state);
            Ok((a, 0))
        }
    }
}

pub fn parse_nth(input: &mut Parser<'_, '_>) -> ParseResult<(i32, i32)> {
    match input.next_token()? {
        Token::Number(n) if n.is_integer => Ok((0, n.int_value().ok_or(ParseError::Invalid)?)),
        Token::Dimension(n, unit) if n.is_integer => {
            parse_n_rest(input, n.int_value().ok_or(ParseError::Invalid)?, &unit)
        }
        Token::Ident(ident) => {
            if ident.eq_ignore_ascii_case("odd") {
                return Ok((2, 1));
            }
            if ident.eq_ignore_ascii_case("even") {
                return Ok((2, 0));
            }
            match ident.strip_prefix('-') {
                Some(rest) => parse_n_rest(input, -1, rest),
                None => parse_n_rest(input, 1, &ident),
            }
        }
        Token::Delim('+') => match input.next_including_whitespace()? {
            Token::Ident(ident) => parse_n_rest(input, 1, &ident),
            _ => Err(ParseError::Invalid),
        },
        _ => Err(ParseError::Invalid),
    }
}
