#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluralRule {
    forms: usize,
    rule: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Expression {
    Count,
    Number(u64),
    Not(Box<Expression>),
    Binary(Operator, Box<Expression>, Box<Expression>),
    Choice(Box<Expression>, Box<Expression>, Box<Expression>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Or,
    And,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}

const LEVELS: [&[(&str, Operator)]; 5] = [
    &[("||", Operator::Or)],
    &[("&&", Operator::And)],
    &[("==", Operator::Equal), ("!=", Operator::NotEqual)],
    &[
        ("<=", Operator::LessOrEqual),
        (">=", Operator::GreaterOrEqual),
        ("<", Operator::Less),
        (">", Operator::Greater),
    ],
    &[("+", Operator::Add), ("-", Operator::Subtract)],
];
const PRODUCTS: &[(&str, Operator)] = &[
    ("*", Operator::Multiply),
    ("/", Operator::Divide),
    ("%", Operator::Remainder),
];

impl Default for PluralRule {
    fn default() -> Self {
        Self {
            forms: 2,
            rule: Expression::Binary(
                Operator::NotEqual,
                Box::new(Expression::Count),
                Box::new(Expression::Number(1)),
            ),
        }
    }
}

impl PluralRule {
    pub fn parse(header: &str) -> Result<Self, String> {
        let mut forms = None;
        let mut rule = None;
        for field in header
            .split(';')
            .map(str::trim)
            .filter(|field| !field.is_empty())
        {
            match field
                .split_once('=')
                .map(|(name, value)| (name.trim(), value.trim()))
            {
                Some(("nplurals", value)) => {
                    forms = Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| format!("nplurals={value} is not a number"))?,
                    );
                }
                Some(("plural", value)) => {
                    let mut parser = Parser { rest: value };
                    let expression = parser.choice()?;
                    if !parser.rest.trim().is_empty() {
                        return Err(format!("cannot read “{}” in the plural rule", parser.rest));
                    }
                    rule = Some(expression);
                }
                _ => return Err(format!("“{field}” is not part of a plural rule")),
            }
        }
        match (forms, rule) {
            (Some(forms), Some(rule)) if forms > 0 => Ok(Self { forms, rule }),
            _ => Err(format!("“{header}” needs nplurals and plural")),
        }
    }

    pub fn forms(&self) -> usize {
        self.forms
    }

    pub fn index(&self, count: u64) -> usize {
        usize::try_from(evaluate(&self.rule, count))
            .unwrap_or(0)
            .min(self.forms - 1)
    }
}

fn evaluate(expression: &Expression, count: u64) -> u64 {
    match expression {
        Expression::Count => count,
        Expression::Number(number) => *number,
        Expression::Not(inner) => u64::from(evaluate(inner, count) == 0),
        Expression::Choice(condition, yes, no) => {
            if evaluate(condition, count) != 0 {
                evaluate(yes, count)
            } else {
                evaluate(no, count)
            }
        }
        Expression::Binary(operator, left, right) => {
            let left = evaluate(left, count);
            let right = evaluate(right, count);
            match operator {
                Operator::Or => u64::from(left != 0 || right != 0),
                Operator::And => u64::from(left != 0 && right != 0),
                Operator::Equal => u64::from(left == right),
                Operator::NotEqual => u64::from(left != right),
                Operator::Less => u64::from(left < right),
                Operator::LessOrEqual => u64::from(left <= right),
                Operator::Greater => u64::from(left > right),
                Operator::GreaterOrEqual => u64::from(left >= right),
                Operator::Add => left.wrapping_add(right),
                Operator::Subtract => left.wrapping_sub(right),
                Operator::Multiply => left.wrapping_mul(right),
                Operator::Divide => left.checked_div(right).unwrap_or(0),
                Operator::Remainder => left.checked_rem(right).unwrap_or(0),
            }
        }
    }
}

struct Parser<'a> {
    rest: &'a str,
}

impl Parser<'_> {
    fn eat(&mut self, token: &str) -> bool {
        let trimmed = self.rest.trim_start();
        match trimmed.strip_prefix(token) {
            Some(rest) => {
                self.rest = rest;
                true
            }
            None => false,
        }
    }

    fn choice(&mut self) -> Result<Expression, String> {
        let condition = self.binary(0)?;
        if !self.eat("?") {
            return Ok(condition);
        }
        let yes = self.choice()?;
        if !self.eat(":") {
            return Err("a ? without its :".to_owned());
        }
        let no = self.choice()?;
        Ok(Expression::Choice(
            Box::new(condition),
            Box::new(yes),
            Box::new(no),
        ))
    }

    fn binary(&mut self, level: usize) -> Result<Expression, String> {
        let operators = LEVELS.get(level).copied().unwrap_or(PRODUCTS);
        let operand = |parser: &mut Self| {
            if level < LEVELS.len() {
                parser.binary(level + 1)
            } else {
                parser.unary()
            }
        };
        let mut left = operand(self)?;
        while let Some(operator) = operators
            .iter()
            .find(|(token, _)| self.eat(token))
            .map(|(_, operator)| *operator)
        {
            let right = operand(self)?;
            left = Expression::Binary(operator, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expression, String> {
        if self.eat("!") {
            return Ok(Expression::Not(Box::new(self.unary()?)));
        }
        if self.eat("(") {
            let inner = self.choice()?;
            if !self.eat(")") {
                return Err("a ( without its )".to_owned());
            }
            return Ok(inner);
        }
        if self.eat("n") {
            return Ok(Expression::Count);
        }
        let trimmed = self.rest.trim_start();
        let digits = trimmed
            .find(|character: char| !character.is_ascii_digit())
            .unwrap_or(trimmed.len());
        let number = trimmed[..digits]
            .parse()
            .map_err(|_| format!("cannot read “{trimmed}” in the plural rule"))?;
        self.rest = &trimmed[digits..];
        Ok(Expression::Number(number))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gettext_rules_pick_the_forms_their_languages_need() {
        let polish = PluralRule::parse(
            "nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
        );
        let forms =
            [1, 2, 5, 12, 22, 25, 102].map(|count| polish.as_ref().map(|rule| rule.index(count)));
        assert_eq!(forms, [0, 1, 2, 2, 1, 2, 1].map(Ok));
        let swedish = PluralRule::parse("nplurals=2; plural=(n != 1);");
        assert_eq!(swedish, Ok(PluralRule::default()));
        assert_eq!(
            PluralRule::parse("nplurals=1; plural=0;").map(|rule| rule.index(7)),
            Ok(0)
        );
        assert!(PluralRule::parse("nplurals=2; plural=(n != 1").is_err());
    }
}
