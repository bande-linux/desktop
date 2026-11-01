use std::f64::consts::{E, PI, TAU};

const SIGNIFICANT_DIGITS: i32 = 12;
const SCIENTIFIC_ABOVE: f64 = 1e15;
const SCIENTIFIC_BELOW: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Length,
    Mass,
    Time,
    Data,
    Temperature,
    Volume,
    Speed,
    Area,
}

#[derive(Debug, PartialEq)]
struct Unit {
    names: &'static [&'static str],
    dimension: Dimension,
    factor: f64,
    offset: f64,
}

const fn unit(names: &'static [&'static str], dimension: Dimension, factor: f64) -> Unit {
    Unit {
        names,
        dimension,
        factor,
        offset: 0.0,
    }
}

const FAHRENHEIT_FACTOR: f64 = 5.0 / 9.0;
const KELVIN_AT_ZERO_CELSIUS: f64 = 273.15;

static UNITS: &[Unit] = &[
    unit(&["m", "metre", "meter"], Dimension::Length, 1.0),
    unit(&["km", "kilometre", "kilometer"], Dimension::Length, 1000.0),
    unit(&["cm", "centimetre", "centimeter"], Dimension::Length, 0.01),
    unit(
        &["mm", "millimetre", "millimeter"],
        Dimension::Length,
        0.001,
    ),
    unit(&["mi", "mile"], Dimension::Length, 1609.344),
    unit(&["yd", "yard"], Dimension::Length, 0.9144),
    unit(&["ft", "foot", "feet"], Dimension::Length, 0.3048),
    unit(&["in", "inch", "inches"], Dimension::Length, 0.0254),
    unit(&["kg", "kilogram", "kilo"], Dimension::Mass, 1.0),
    unit(&["g", "gram"], Dimension::Mass, 0.001),
    unit(&["mg", "milligram"], Dimension::Mass, 1e-6),
    unit(&["t", "tonne", "ton"], Dimension::Mass, 1000.0),
    unit(&["lb", "lbs", "pound"], Dimension::Mass, 0.453_592_37),
    unit(&["oz", "ounce"], Dimension::Mass, 0.028_349_523_125),
    unit(&["st", "stone"], Dimension::Mass, 6.350_293_18),
    unit(&["s", "sec", "second"], Dimension::Time, 1.0),
    unit(&["ms", "millisecond"], Dimension::Time, 0.001),
    unit(&["min", "minute"], Dimension::Time, 60.0),
    unit(&["h", "hr", "hour"], Dimension::Time, 3600.0),
    unit(&["d", "day"], Dimension::Time, 86_400.0),
    unit(&["wk", "week"], Dimension::Time, 604_800.0),
    unit(&["yr", "year"], Dimension::Time, 31_557_600.0),
    unit(&["B", "byte"], Dimension::Data, 1.0),
    unit(&["KB", "kilobyte"], Dimension::Data, 1e3),
    unit(&["MB", "megabyte"], Dimension::Data, 1e6),
    unit(&["GB", "gigabyte"], Dimension::Data, 1e9),
    unit(&["TB", "terabyte"], Dimension::Data, 1e12),
    unit(&["KiB", "kibibyte"], Dimension::Data, 1024.0),
    unit(&["MiB", "mebibyte"], Dimension::Data, 1_048_576.0),
    unit(&["GiB", "gibibyte"], Dimension::Data, 1_073_741_824.0),
    unit(&["TiB", "tebibyte"], Dimension::Data, 1_099_511_627_776.0),
    Unit {
        names: &["°C", "c", "celsius"],
        dimension: Dimension::Temperature,
        factor: 1.0,
        offset: KELVIN_AT_ZERO_CELSIUS,
    },
    Unit {
        names: &["°F", "f", "fahrenheit"],
        dimension: Dimension::Temperature,
        factor: FAHRENHEIT_FACTOR,
        offset: KELVIN_AT_ZERO_CELSIUS - 32.0 * FAHRENHEIT_FACTOR,
    },
    unit(&["K", "kelvin"], Dimension::Temperature, 1.0),
    unit(&["l", "litre", "liter"], Dimension::Volume, 1.0),
    unit(
        &["ml", "millilitre", "milliliter"],
        Dimension::Volume,
        0.001,
    ),
    unit(&["cl", "centilitre", "centiliter"], Dimension::Volume, 0.01),
    unit(&["dl", "decilitre", "deciliter"], Dimension::Volume, 0.1),
    unit(&["gal", "gallon"], Dimension::Volume, 3.785_411_784),
    unit(&["qt", "quart"], Dimension::Volume, 0.946_352_946),
    unit(&["pt", "pint"], Dimension::Volume, 0.473_176_473),
    unit(&["cup"], Dimension::Volume, 0.236_588_236_5),
    unit(&["floz"], Dimension::Volume, 0.029_573_529_562_5),
    unit(
        &["tbsp", "tablespoon"],
        Dimension::Volume,
        0.014_786_764_781_25,
    ),
    unit(
        &["tsp", "teaspoon"],
        Dimension::Volume,
        0.004_928_921_593_75,
    ),
    unit(&["m²", "sqm"], Dimension::Area, 1.0),
    unit(&["km²", "sqkm"], Dimension::Area, 1e6),
    unit(&["cm²", "sqcm"], Dimension::Area, 1e-4),
    unit(&["ha", "hectare"], Dimension::Area, 1e4),
    unit(&["ac", "acre"], Dimension::Area, 4_046.856_422_4),
    unit(&["mi²", "sqmi"], Dimension::Area, 2_589_988.110_336),
    unit(&["yd²", "sqyd"], Dimension::Area, 0.836_127_36),
    unit(&["ft²", "sqft"], Dimension::Area, 0.092_903_04),
    unit(&["in²", "sqin"], Dimension::Area, 0.000_645_16),
    unit(&["m/s"], Dimension::Speed, 1.0),
    unit(&["km/h", "kmh", "kph"], Dimension::Speed, 1.0 / 3.6),
    unit(&["mph"], Dimension::Speed, 0.447_04),
    unit(&["kn", "knot"], Dimension::Speed, 0.514_444_444_444),
];

fn find_unit(name: &str) -> Option<&'static Unit> {
    let lookup = |name: &str| {
        UNITS.iter().find(|unit| {
            unit.names
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(name))
        })
    };
    lookup(name).or_else(|| name.strip_suffix('s').and_then(lookup))
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Word(String),
    Symbol(char),
}

fn number(chars: &[char], start: usize) -> (f64, usize) {
    let mut text = String::new();
    let mut index = start;
    while index < chars.len() {
        let character = chars[index];
        let grouped = character == ','
            && chars.len() >= index + 4
            && chars[index + 1..index + 4].iter().all(char::is_ascii_digit)
            && chars
                .get(index + 4)
                .is_none_or(|next| !next.is_ascii_digit());
        let exponent = (character == 'e' || character == 'E')
            && chars.get(index + 1).is_some_and(char::is_ascii_digit);
        if character.is_ascii_digit() || character == '.' {
            text.push(character);
        } else if exponent {
            text.push('e');
        } else if !grouped {
            break;
        }
        index += 1;
    }
    (text.parse().unwrap_or(f64::NAN), index)
}

fn tokens(text: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character.is_whitespace() {
            index += 1;
        } else if character.is_ascii_digit() || character == '.' {
            let (value, end) = number(&chars, index);
            if value.is_nan() {
                return None;
            }
            tokens.push(Token::Number(value));
            index = end;
        } else if character.is_alphabetic() || character == '°' {
            let mut end = index + 1;
            while end < chars.len() && (chars[end].is_alphabetic() || chars[end] == '²') {
                end += 1;
            }
            if chars.get(end) == Some(&'/') {
                let mut joined = end + 1;
                while joined < chars.len() && chars[joined].is_alphabetic() {
                    joined += 1;
                }
                let candidate: String = chars[index..joined].iter().collect();
                if joined > end + 1 && find_unit(&candidate).is_some() {
                    end = joined;
                }
            }
            tokens.push(Token::Word(chars[index..end].iter().collect()));
            index = end;
        } else if "+-*/^()%×÷−·".contains(character) {
            tokens.push(Token::Symbol(match character {
                '×' | '·' => '*',
                '÷' => '/',
                '−' => '-',
                other => other,
            }));
            index += 1;
        } else {
            return None;
        }
    }
    Some(tokens)
}

#[derive(Debug, Clone, Copy)]
struct Value {
    amount: f64,
    unit: Option<&'static Unit>,
    percent: bool,
}

impl Value {
    const fn scalar(amount: f64) -> Self {
        Self {
            amount,
            unit: None,
            percent: false,
        }
    }

    fn convert(self, target: &'static Unit) -> Option<Self> {
        let unit = self.unit?;
        if unit.dimension != target.dimension {
            return None;
        }
        let base = self.amount * unit.factor + unit.offset;
        Some(Self {
            amount: (base - target.offset) / target.factor,
            unit: Some(target),
            percent: false,
        })
    }

    fn matched(self, other: Self) -> Option<Self> {
        match (self.unit, other.unit) {
            (None, None) => Some(other),
            (Some(unit), Some(_)) => other.convert(unit),
            _ => None,
        }
    }
}

const MAX_TOKENS: usize = 256;

struct Parser {
    tokens: Vec<Token>,
    position: usize,
    operations: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn eat(&mut self, symbol: char) -> bool {
        if self.peek() == Some(&Token::Symbol(symbol)) {
            self.position += 1;
            return true;
        }
        false
    }

    fn eat_word(&mut self, word: &str) -> bool {
        if matches!(self.peek(), Some(Token::Word(found)) if found.eq_ignore_ascii_case(word)) {
            self.position += 1;
            return true;
        }
        false
    }

    fn sum(&mut self) -> Option<Value> {
        let mut left = self.product()?;
        loop {
            let sign = if self.eat('+') {
                1.0
            } else if self.eat('-') {
                -1.0
            } else {
                return Some(left);
            };
            self.operations += 1;
            let right = self.product()?;
            left = if right.percent && !left.percent {
                Value {
                    amount: left.amount * (1.0 + sign * right.amount),
                    ..left
                }
            } else {
                let right = left.matched(right)?;
                Value {
                    amount: left.amount + sign * right.amount,
                    percent: false,
                    ..left
                }
            };
        }
    }

    fn product(&mut self) -> Option<Value> {
        let mut left = self.unary()?;
        loop {
            let multiply = if self.eat('*') || self.eat_word("x") || self.eat_word("of") {
                true
            } else if self.eat('/') {
                false
            } else {
                return Some(left);
            };
            self.operations += 1;
            let right = self.unary()?;
            left = match (multiply, left.unit, right.unit) {
                (true, None, unit) | (true, unit, None) => Value {
                    amount: left.amount * right.amount,
                    unit,
                    percent: false,
                },
                (false, unit, None) => Value {
                    amount: left.amount / right.amount,
                    unit,
                    percent: false,
                },
                (false, Some(_), Some(_)) => {
                    let right = left.matched(right)?;
                    Value::scalar(left.amount / right.amount)
                }
                _ => return None,
            };
        }
    }

    fn unary(&mut self) -> Option<Value> {
        if self.eat('-') {
            let value = self.unary()?;
            return Some(Value {
                amount: -value.amount,
                ..value
            });
        }
        if self.eat('+') {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<Value> {
        let base = self.postfix()?;
        if !self.eat('^') {
            return Some(base);
        }
        self.operations += 1;
        let exponent = self.unary()?;
        if base.unit.is_some() || exponent.unit.is_some() {
            return None;
        }
        Some(Value::scalar(base.amount.powf(exponent.amount)))
    }

    fn postfix(&mut self) -> Option<Value> {
        let value = self.primary()?;
        if self.eat('%') {
            if value.unit.is_some() {
                return None;
            }
            self.operations += 1;
            return Some(Value {
                amount: value.amount / 100.0,
                unit: None,
                percent: true,
            });
        }
        Some(value)
    }

    fn with_unit(&mut self, value: Value) -> Value {
        if value.unit.is_none()
            && let Some(Token::Word(word)) = self.peek()
            && let Some(unit) = find_unit(word)
        {
            self.position += 1;
            return Value {
                unit: Some(unit),
                ..value
            };
        }
        value
    }

    fn primary(&mut self) -> Option<Value> {
        let token = self.peek()?.clone();
        self.position += 1;
        match token {
            Token::Number(amount) => Some(self.with_unit(Value::scalar(amount))),
            Token::Symbol('(') => {
                let value = self.sum()?;
                self.eat(')');
                Some(self.with_unit(value))
            }
            Token::Word(word) => {
                self.operations += 1;
                let word = word.to_lowercase();
                let constant = match word.as_str() {
                    "pi" | "π" => Some(PI),
                    "e" => Some(E),
                    "tau" | "τ" => Some(TAU),
                    _ => None,
                };
                if let Some(constant) = constant {
                    return Some(Value::scalar(constant));
                }
                let function: fn(f64) -> f64 = match word.as_str() {
                    "sqrt" => f64::sqrt,
                    "abs" => f64::abs,
                    "ln" => f64::ln,
                    "log" => f64::log10,
                    "exp" => f64::exp,
                    "sin" => f64::sin,
                    "cos" => f64::cos,
                    "tan" => f64::tan,
                    "round" => f64::round,
                    "floor" => f64::floor,
                    "ceil" => f64::ceil,
                    _ => return None,
                };
                let argument = self.postfix()?;
                if argument.unit.is_some() {
                    return None;
                }
                Some(Value::scalar(function(argument.amount)))
            }
            Token::Symbol(_) => None,
        }
    }
}

fn split_conversion(text: &str) -> Option<(&str, &'static Unit)> {
    [" in ", " to ", " as "]
        .iter()
        .filter_map(|keyword| {
            let folded = text.to_ascii_lowercase();
            let at = folded.rfind(keyword)?;
            let target = find_unit(text.get(at + keyword.len()..)?.trim())?;
            Some((at, &text[..at], target))
        })
        .max_by_key(|(at, _, _)| *at)
        .map(|(_, expression, target)| (expression, target))
}

pub fn format_number(amount: f64) -> String {
    if amount == 0.0 {
        return "0".to_owned();
    }
    let magnitude = amount.abs();
    if !(SCIENTIFIC_BELOW..SCIENTIFIC_ABOVE).contains(&magnitude) {
        let text = format!("{:.*e}", (SIGNIFICANT_DIGITS - 1) as usize, amount);
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{mantissa}e{exponent}");
    }
    let whole_digits = magnitude.log10().floor() as i32 + 1;
    let decimals = (SIGNIFICANT_DIGITS - whole_digits).max(0) as usize;
    let text = format!("{amount:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calculation {
    pub value: String,
    pub conversion: bool,
}

pub fn calculate(text: &str) -> Option<Calculation> {
    let (expression, target) = match split_conversion(text) {
        Some((expression, target)) => (expression, Some(target)),
        None => (text, None),
    };
    let mut parser = Parser {
        tokens: tokens(expression).filter(|tokens| tokens.len() <= MAX_TOKENS)?,
        position: 0,
        operations: 0,
    };
    let mut value = parser.sum()?;
    if parser.position != parser.tokens.len() {
        return None;
    }
    if let Some(target) = target {
        value = value.convert(target)?;
        parser.operations += 1;
    }
    if parser.operations == 0 || !value.amount.is_finite() {
        return None;
    }
    let number = format_number(value.amount);
    Some(Calculation {
        conversion: target.is_some() || value.unit.is_some(),
        value: match value.unit {
            Some(unit) if unit.names[0].starts_with('°') => format!("{number}{}", unit.names[0]),
            Some(unit) => format!("{number} {}", unit.names[0]),
            None => number,
        },
    })
}
