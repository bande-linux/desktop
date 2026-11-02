use crate::parser::{ParseError, ParseResult, Parser};
use crate::tokenizer::Token;
use crate::values::{Length, LengthUnit, angle_to_degrees, resolution_to_dppx, time_to_seconds};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CalcLeaf {
    Number(f32),
    Percentage(f32),
    Length(Length),
    Angle(f32),
    Time(f32),
    Resolution(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Number,
    Length,
    Percentage,
    Angle,
    Time,
    Resolution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoundingStrategy {
    Nearest,
    Up,
    Down,
    ToZero,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CalcNode {
    Leaf(CalcLeaf),
    Sum(Box<[CalcNode]>),
    Product(Box<[CalcNode]>),
    Negate(Box<CalcNode>),
    Invert(Box<CalcNode>),
    Min(Box<[CalcNode]>),
    Max(Box<[CalcNode]>),
    Clamp(Box<[CalcNode; 3]>),
    Round(RoundingStrategy, Box<[CalcNode; 2]>),
    Mod(Box<[CalcNode; 2]>),
    Rem(Box<[CalcNode; 2]>),
    Hypot(Box<[CalcNode]>),
    Abs(Box<CalcNode>),
    Sign(Box<CalcNode>),
}

impl CalcLeaf {
    fn category(&self) -> Category {
        match self {
            CalcLeaf::Number(_) => Category::Number,
            CalcLeaf::Percentage(_) => Category::Percentage,
            CalcLeaf::Length(_) => Category::Length,
            CalcLeaf::Angle(_) => Category::Angle,
            CalcLeaf::Time(_) => Category::Time,
            CalcLeaf::Resolution(_) => Category::Resolution,
        }
    }

    fn resolved(&self) -> Option<f32> {
        match *self {
            CalcLeaf::Number(v)
            | CalcLeaf::Angle(v)
            | CalcLeaf::Time(v)
            | CalcLeaf::Resolution(v) => Some(v),
            CalcLeaf::Length(length) => length.to_px(),
            CalcLeaf::Percentage(_) => None,
        }
    }

    fn with_value(&self, value: f32) -> CalcLeaf {
        match *self {
            CalcLeaf::Number(_) => CalcLeaf::Number(value),
            CalcLeaf::Percentage(_) => CalcLeaf::Percentage(value),
            CalcLeaf::Length(_) => CalcLeaf::Length(Length {
                value,
                unit: LengthUnit::Px,
            }),
            CalcLeaf::Angle(_) => CalcLeaf::Angle(value),
            CalcLeaf::Time(_) => CalcLeaf::Time(value),
            CalcLeaf::Resolution(_) => CalcLeaf::Resolution(value),
        }
    }
}

fn sanitize(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(f32::MIN, f32::MAX)
    }
}

impl CalcNode {
    pub fn evaluate(&self, resolve: &dyn Fn(&CalcLeaf) -> Option<f32>) -> Option<f32> {
        self.eval(resolve).map(sanitize)
    }

    fn eval(&self, resolve: &dyn Fn(&CalcLeaf) -> Option<f32>) -> Option<f32> {
        let all = |nodes: &[CalcNode]| -> Option<Vec<f32>> {
            nodes.iter().map(|n| n.eval(resolve)).collect()
        };
        Some(match self {
            CalcNode::Leaf(leaf) => leaf.resolved().or_else(|| resolve(leaf))?,
            CalcNode::Sum(nodes) => all(nodes)?.into_iter().sum(),
            CalcNode::Product(nodes) => all(nodes)?.into_iter().product(),
            CalcNode::Negate(node) => -node.eval(resolve)?,
            CalcNode::Invert(node) => 1.0 / node.eval(resolve)?,
            CalcNode::Min(nodes) => all(nodes)?.into_iter().fold(f32::INFINITY, f32::min),
            CalcNode::Max(nodes) => all(nodes)?.into_iter().fold(f32::NEG_INFINITY, f32::max),
            CalcNode::Clamp(nodes) => {
                let [min, value, max] = [
                    nodes[0].eval(resolve)?,
                    nodes[1].eval(resolve)?,
                    nodes[2].eval(resolve)?,
                ];
                value.min(max).max(min)
            }
            CalcNode::Round(strategy, nodes) => {
                let (value, step) = (nodes[0].eval(resolve)?, nodes[1].eval(resolve)?);
                round(*strategy, value, step)
            }
            CalcNode::Mod(nodes) => {
                let (a, b) = (nodes[0].eval(resolve)?, nodes[1].eval(resolve)?);
                a - b * (a / b).floor()
            }
            CalcNode::Rem(nodes) => {
                let (a, b) = (nodes[0].eval(resolve)?, nodes[1].eval(resolve)?);
                a % b
            }
            CalcNode::Hypot(nodes) => all(nodes)?.into_iter().map(|v| v * v).sum::<f32>().sqrt(),
            CalcNode::Abs(node) => node.eval(resolve)?.abs(),
            CalcNode::Sign(node) => {
                let v = node.eval(resolve)?;
                if v == 0.0 { v } else { v.signum() }
            }
        })
    }

    fn leaf(&self) -> Option<&CalcLeaf> {
        match self {
            CalcNode::Leaf(leaf) => Some(leaf),
            _ => None,
        }
    }

    pub fn as_leaf(&self) -> Option<CalcLeaf> {
        self.leaf().copied()
    }

    fn simplify(self, category: Category) -> CalcNode {
        if let CalcNode::Leaf(_) = self {
            return self;
        }
        if let Some(value) = self.eval(&|_| None) {
            let leaf = match category {
                Category::Number => CalcLeaf::Number(value),
                Category::Length => CalcLeaf::Length(Length {
                    value,
                    unit: LengthUnit::Px,
                }),
                Category::Percentage => CalcLeaf::Percentage(value),
                Category::Angle => CalcLeaf::Angle(value),
                Category::Time => CalcLeaf::Time(value),
                Category::Resolution => CalcLeaf::Resolution(value),
            };
            return CalcNode::Leaf(leaf.with_value(sanitize(value)));
        }
        match self {
            CalcNode::Sum(nodes) => {
                let mut merged: Vec<CalcNode> = Vec::with_capacity(nodes.len());
                for node in nodes.into_vec() {
                    let flat = match node {
                        CalcNode::Sum(inner) => inner.into_vec(),
                        other => vec![other],
                    };
                    for node in flat {
                        if let Some(leaf) = node.leaf().copied()
                            && let Some(existing) = merged
                                .iter_mut()
                                .find(|m| m.leaf().is_some_and(|m| same_unit(m, &leaf)))
                            && let Some(existing_leaf) = existing.leaf().copied()
                        {
                            *existing = CalcNode::Leaf(add_leaves(&existing_leaf, &leaf));
                            continue;
                        }
                        merged.push(node);
                    }
                }
                if merged.len() == 1 {
                    merged
                        .pop()
                        .unwrap_or(CalcNode::Leaf(CalcLeaf::Number(0.0)))
                } else {
                    CalcNode::Sum(merged.into_boxed_slice())
                }
            }
            CalcNode::Product(nodes) => {
                let mut factor = 1.0;
                let mut rest = vec![];
                for node in nodes.into_vec() {
                    match node.leaf() {
                        Some(CalcLeaf::Number(v)) => factor *= v,
                        _ => rest.push(node),
                    }
                }
                if let [single] = &mut rest[..]
                    && let Some(leaf) = single.leaf().copied()
                {
                    let value = match leaf {
                        CalcLeaf::Length(l) => l.value,
                        CalcLeaf::Percentage(v) => v,
                        _ => return CalcNode::Product(rest.into_boxed_slice()),
                    };
                    return CalcNode::Leaf(match leaf {
                        CalcLeaf::Length(l) => CalcLeaf::Length(Length {
                            value: value * factor,
                            unit: l.unit,
                        }),
                        _ => CalcLeaf::Percentage(value * factor),
                    });
                }
                if factor != 1.0 {
                    rest.push(CalcNode::Leaf(CalcLeaf::Number(factor)));
                }
                CalcNode::Product(rest.into_boxed_slice())
            }
            CalcNode::Negate(node) => match node.leaf() {
                Some(CalcLeaf::Length(l)) => CalcNode::Leaf(CalcLeaf::Length(Length {
                    value: -l.value,
                    unit: l.unit,
                })),
                Some(CalcLeaf::Percentage(v)) => CalcNode::Leaf(CalcLeaf::Percentage(-v)),
                _ => CalcNode::Negate(node),
            },
            other => other,
        }
    }
}

fn same_unit(a: &CalcLeaf, b: &CalcLeaf) -> bool {
    match (a, b) {
        (CalcLeaf::Length(a), CalcLeaf::Length(b)) => a.unit == b.unit,
        _ => a.category() == b.category() && a.resolved().is_some() == b.resolved().is_some(),
    }
}

fn add_leaves(a: &CalcLeaf, b: &CalcLeaf) -> CalcLeaf {
    match (a, b) {
        (CalcLeaf::Length(a), CalcLeaf::Length(b)) => CalcLeaf::Length(Length {
            value: a.value + b.value,
            unit: a.unit,
        }),
        (CalcLeaf::Percentage(a), CalcLeaf::Percentage(b)) => CalcLeaf::Percentage(a + b),
        _ => a.with_value(a.resolved().unwrap_or(0.0) + b.resolved().unwrap_or(0.0)),
    }
}

fn round(strategy: RoundingStrategy, value: f32, step: f32) -> f32 {
    if step == 0.0 {
        return f32::NAN;
    }
    let quotient = value / step;
    let rounded = match strategy {
        RoundingStrategy::Nearest => (quotient + 0.5).floor(),
        RoundingStrategy::Up => quotient.ceil(),
        RoundingStrategy::Down => quotient.floor(),
        RoundingStrategy::ToZero => quotient.trunc(),
    };
    rounded * step
}

pub fn is_math_function(name: &str) -> bool {
    MATH_FUNCTIONS.iter().any(|f| f.eq_ignore_ascii_case(name))
}

const MATH_FUNCTIONS: &[&str] = &[
    "calc", "min", "max", "clamp", "round", "mod", "rem", "sin", "cos", "tan", "asin", "acos",
    "atan", "atan2", "pow", "sqrt", "hypot", "log", "exp", "abs", "sign",
];

pub struct CalcParser {
    pub percentage_as: Option<Category>,
}

type Typed = (CalcNode, Category);

impl CalcParser {
    pub fn parse_function(
        &self,
        name: &str,
        input: &mut Parser<'_, '_>,
        expected: Category,
    ) -> ParseResult<CalcNode> {
        let (node, category) = input.parse_nested_block(|p| self.function(name, p))?;
        let accepted = category == expected
            || (category == Category::Percentage && self.percentage_as == Some(expected))
            || (category == Category::Number && expected == Category::Number);
        if !accepted {
            return Err(ParseError::Invalid);
        }
        Ok(node.simplify(expected))
    }

    pub fn parse_function_any(
        &self,
        name: &str,
        input: &mut Parser<'_, '_>,
    ) -> ParseResult<(CalcNode, Category)> {
        let (node, category) = input.parse_nested_block(|p| self.function(name, p))?;
        Ok((node.simplify(category), category))
    }

    fn combine(&self, a: Category, b: Category) -> ParseResult<Category> {
        match (a, b) {
            _ if a == b => Ok(a),
            (Category::Percentage, other) | (other, Category::Percentage)
                if self.percentage_as == Some(other) =>
            {
                Ok(other)
            }
            _ => Err(ParseError::Invalid),
        }
    }

    fn function(&self, name: &str, input: &mut Parser<'_, '_>) -> ParseResult<Typed> {
        let name = name.to_ascii_lowercase();
        let result = match name.as_str() {
            "calc" => self.sum(input)?,
            "min" | "max" | "hypot" => {
                let args = self.args(input)?;
                let category = self.same_category(&args)?;
                let nodes: Box<[CalcNode]> = args.into_iter().map(|(n, _)| n).collect();
                let node = match name.as_str() {
                    "min" => CalcNode::Min(nodes),
                    "max" => CalcNode::Max(nodes),
                    _ => CalcNode::Hypot(nodes),
                };
                (node, category)
            }
            "clamp" => {
                let [a, b, c] = self.fixed_args::<3>(input)?;
                let category = self.same_category(&[a.clone(), b.clone(), c.clone()])?;
                (CalcNode::Clamp(Box::new([a.0, b.0, c.0])), category)
            }
            "round" => {
                let strategy = match input.try_parse(|p| {
                    let ident = p.expect_ident()?;
                    p.expect_comma()?;
                    Ok(ident)
                }) {
                    Ok(s) if s.eq_ignore_ascii_case("nearest") => RoundingStrategy::Nearest,
                    Ok(s) if s.eq_ignore_ascii_case("up") => RoundingStrategy::Up,
                    Ok(s) if s.eq_ignore_ascii_case("down") => RoundingStrategy::Down,
                    Ok(s) if s.eq_ignore_ascii_case("to-zero") => RoundingStrategy::ToZero,
                    Ok(_) => return Err(ParseError::Invalid),
                    Err(_) => RoundingStrategy::Nearest,
                };
                let args = self.args(input)?;
                let category = self.same_category(&args)?;
                let mut nodes = args.into_iter().map(|(n, _)| n);
                let value = nodes.next().ok_or(ParseError::Invalid)?;
                let step = match (nodes.next(), category) {
                    (Some(step), _) => step,
                    (None, Category::Number) => CalcNode::Leaf(CalcLeaf::Number(1.0)),
                    (None, _) => return Err(ParseError::Invalid),
                };
                if nodes.next().is_some() {
                    return Err(ParseError::Invalid);
                }
                (CalcNode::Round(strategy, Box::new([value, step])), category)
            }
            "mod" | "rem" => {
                let [a, b] = self.fixed_args::<2>(input)?;
                let category = self.combine(a.1, b.1)?;
                let pair = Box::new([a.0, b.0]);
                let node = if name == "mod" {
                    CalcNode::Mod(pair)
                } else {
                    CalcNode::Rem(pair)
                };
                (node, category)
            }
            "abs" => {
                let (node, category) = self.sum(input)?;
                (CalcNode::Abs(Box::new(node)), category)
            }
            "sign" => {
                let (node, _) = self.sum(input)?;
                (CalcNode::Sign(Box::new(node)), Category::Number)
            }
            "sin" | "cos" | "tan" => {
                let (node, category) = self.sum(input)?;
                let value = resolved(&node)?;
                let radians = match category {
                    Category::Number => value,
                    Category::Angle => value.to_radians(),
                    _ => return Err(ParseError::Invalid),
                };
                let result = match name.as_str() {
                    "sin" => radians.sin(),
                    "cos" => radians.cos(),
                    _ => radians.tan(),
                };
                number(result)
            }
            "asin" | "acos" | "atan" => {
                let (node, category) = self.sum(input)?;
                if category != Category::Number {
                    return Err(ParseError::Invalid);
                }
                let value = resolved(&node)?;
                let radians = match name.as_str() {
                    "asin" => value.asin(),
                    "acos" => value.acos(),
                    _ => value.atan(),
                };
                (
                    CalcNode::Leaf(CalcLeaf::Angle(radians.to_degrees())),
                    Category::Angle,
                )
            }
            "atan2" => {
                let [a, b] = self.fixed_args::<2>(input)?;
                self.combine(a.1, b.1)?;
                let angle = resolved(&a.0)?.atan2(resolved(&b.0)?).to_degrees();
                (CalcNode::Leaf(CalcLeaf::Angle(angle)), Category::Angle)
            }
            "pow" => {
                let [a, b] = self.fixed_args::<2>(input)?;
                numbers(&[&a, &b])?;
                number(resolved(&a.0)?.powf(resolved(&b.0)?))
            }
            "sqrt" | "exp" => {
                let a = self.sum(input)?;
                numbers(&[&a])?;
                let v = resolved(&a.0)?;
                number(if name == "sqrt" { v.sqrt() } else { v.exp() })
            }
            "log" => {
                let args = self.args(input)?;
                let values = args
                    .iter()
                    .map(|a| {
                        numbers(&[a])?;
                        resolved(&a.0)
                    })
                    .collect::<ParseResult<Vec<_>>>()?;
                match values[..] {
                    [v] => number(v.ln()),
                    [v, base] => number(v.ln() / base.ln()),
                    _ => return Err(ParseError::Invalid),
                }
            }
            _ => return Err(ParseError::Invalid),
        };
        input.expect_exhausted()?;
        Ok(result)
    }

    fn args(&self, input: &mut Parser<'_, '_>) -> ParseResult<Vec<Typed>> {
        input.parse_comma_separated(|p| self.sum(p))
    }

    fn fixed_args<const N: usize>(&self, input: &mut Parser<'_, '_>) -> ParseResult<[Typed; N]> {
        self.args(input)?
            .try_into()
            .map_err(|_| ParseError::Invalid)
    }

    fn same_category(&self, args: &[Typed]) -> ParseResult<Category> {
        let mut iter = args.iter();
        let first = iter.next().ok_or(ParseError::Invalid)?.1;
        iter.try_fold(first, |acc, (_, c)| self.combine(acc, *c))
    }

    fn sum(&self, input: &mut Parser<'_, '_>) -> ParseResult<Typed> {
        input.skip_whitespace();
        let (first, mut category) = self.product(input)?;
        let mut terms = vec![first];
        loop {
            let state = input.state();
            let had_space = input.next_including_whitespace() == Ok(Token::Whitespace);
            let operator = match input.next_including_whitespace() {
                Ok(Token::Delim(c @ ('+' | '-'))) if had_space => c,
                _ => {
                    input.reset(&state);
                    input.skip_whitespace();
                    break;
                }
            };
            if input.next_including_whitespace() != Ok(Token::Whitespace) {
                return Err(ParseError::Invalid);
            }
            let (term, term_category) = self.product(input)?;
            category = self.combine(category, term_category)?;
            terms.push(if operator == '-' {
                CalcNode::Negate(Box::new(term))
            } else {
                term
            });
        }
        let node = if terms.len() == 1 {
            terms.pop().unwrap_or(CalcNode::Leaf(CalcLeaf::Number(0.0)))
        } else {
            CalcNode::Sum(terms.into_boxed_slice())
        };
        Ok((node, category))
    }

    fn product(&self, input: &mut Parser<'_, '_>) -> ParseResult<Typed> {
        let (first, mut category) = self.value(input)?;
        let mut factors = vec![first];
        loop {
            let state = input.state();
            let operator = match input.next_token() {
                Ok(Token::Delim(c @ ('*' | '/'))) => c,
                _ => {
                    input.reset(&state);
                    break;
                }
            };
            input.skip_whitespace();
            let (factor, factor_category) = self.value(input)?;
            if operator == '*' {
                category = match (category, factor_category) {
                    (Category::Number, other) => other,
                    (other, Category::Number) => other,
                    _ => return Err(ParseError::Invalid),
                };
                factors.push(factor);
            } else {
                if factor_category != Category::Number {
                    return Err(ParseError::Invalid);
                }
                factors.push(CalcNode::Invert(Box::new(factor)));
            }
        }
        let node = if factors.len() == 1 {
            factors
                .pop()
                .unwrap_or(CalcNode::Leaf(CalcLeaf::Number(1.0)))
        } else {
            CalcNode::Product(factors.into_boxed_slice())
        };
        Ok((node, category))
    }

    fn value(&self, input: &mut Parser<'_, '_>) -> ParseResult<Typed> {
        let leaf = match input.next_token()? {
            Token::Number(n) => CalcLeaf::Number(n.value as f32),
            Token::Percentage(n) => {
                if self.percentage_as.is_none() {
                    return Err(ParseError::Invalid);
                }
                CalcLeaf::Percentage(n.value as f32)
            }
            Token::Dimension(n, unit) => dimension_leaf(n.value as f32, &unit)?,
            Token::Ident(ident) => {
                let value = match ident.to_ascii_lowercase().as_str() {
                    "e" => std::f32::consts::E,
                    "pi" => std::f32::consts::PI,
                    "infinity" => f32::INFINITY,
                    "-infinity" => f32::NEG_INFINITY,
                    "nan" => f32::NAN,
                    _ => return Err(ParseError::Invalid),
                };
                CalcLeaf::Number(value)
            }
            Token::OpenParen => {
                return input.parse_nested_block(|p| {
                    let result = self.sum(p)?;
                    p.expect_exhausted()?;
                    Ok(result)
                });
            }
            Token::Function(name) if is_math_function(&name) => {
                return input.parse_nested_block(|p| self.function(&name, p));
            }
            _ => return Err(ParseError::Invalid),
        };
        Ok((CalcNode::Leaf(leaf), leaf.category()))
    }
}

fn resolved(node: &CalcNode) -> ParseResult<f32> {
    node.eval(&|_| None).ok_or(ParseError::Invalid)
}

fn number(value: f32) -> Typed {
    (CalcNode::Leaf(CalcLeaf::Number(value)), Category::Number)
}

fn numbers(args: &[&Typed]) -> ParseResult<()> {
    if args.iter().all(|a| a.1 == Category::Number) {
        Ok(())
    } else {
        Err(ParseError::Invalid)
    }
}

pub fn dimension_leaf(value: f32, unit: &str) -> ParseResult<CalcLeaf> {
    if let Some(unit) = LengthUnit::from_name(unit) {
        let length = Length { value, unit };
        return Ok(CalcLeaf::Length(match length.to_px() {
            Some(px) => Length {
                value: px,
                unit: LengthUnit::Px,
            },
            None => length,
        }));
    }
    if let Some(degrees) = angle_to_degrees(value, unit) {
        return Ok(CalcLeaf::Angle(degrees));
    }
    if let Some(seconds) = time_to_seconds(value, unit) {
        return Ok(CalcLeaf::Time(seconds));
    }
    if let Some(dppx) = resolution_to_dppx(value, unit) {
        return Ok(CalcLeaf::Resolution(dppx));
    }
    Err(ParseError::Invalid)
}
