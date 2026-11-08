use proc_macro2::{Delimiter, TokenStream, TokenTree};

use crate::pattern::placeholders;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Translate,
    Plural,
}

impl Kind {
    pub fn of_macro(name: &str) -> Option<Self> {
        match name {
            "text" => Some(Self::Text),
            "tr" => Some(Self::Translate),
            "trn" => Some(Self::Plural),
            _ => None,
        }
    }
}

pub struct Invocation {
    pub kind: Kind,
    pub context: Option<String>,
    pub english: String,
    pub plural: Option<String>,
    pub args: Vec<(String, TokenStream)>,
}

impl Invocation {
    pub fn parse(kind: Kind, input: TokenStream) -> Result<Self, String> {
        let mut parts = split_commas(input).into_iter();
        let mut first = parts.next().ok_or("needs the English text")?;
        let context = match named(&first) {
            Some((name, value)) if name == "context" => {
                let context = literal(&value)?;
                first = parts
                    .next()
                    .ok_or("needs the English text after the context")?;
                Some(context)
            }
            _ => None,
        };
        let english = literal(&first)?;
        let plural = match kind {
            Kind::Plural => Some(literal(&parts.next().ok_or("needs the English plural")?)?),
            _ => None,
        };
        let args = parts
            .map(|part| named(&part).ok_or("arguments are written name = value".to_owned()))
            .collect::<Result<Vec<_>, _>>()?;
        let invocation = Self {
            kind,
            context,
            english,
            plural,
            args,
        };
        invocation.validate()?;
        Ok(invocation)
    }

    fn validate(&self) -> Result<(), String> {
        if self.english.trim().is_empty() {
            return Err("the English text is empty".to_owned());
        }
        let mut used = placeholders(&self.english)?;
        if let Some(plural) = &self.plural {
            used.extend(placeholders(plural)?);
        }
        if self.kind == Kind::Text {
            return Ok(());
        }
        if self.kind == Kind::Plural && self.args.is_empty() {
            return Err("needs the count as its first argument".to_owned());
        }
        for name in &used {
            if !self.args.iter().any(|(arg, _)| arg == name) {
                return Err(format!("{{{name}}} has no argument"));
            }
        }
        let counted = usize::from(self.kind == Kind::Plural);
        for (name, _) in self.args.iter().skip(counted) {
            if !used.contains(&name.as_str()) {
                return Err(format!("{name} is not used by the English"));
            }
        }
        Ok(())
    }
}

pub fn invocations(source: TokenStream, found: &mut Vec<Invocation>, errors: &mut Vec<String>) {
    let tokens = source.into_iter().collect::<Vec<_>>();
    let mut index = 0;
    while index < tokens.len() {
        if is_test_attribute(&tokens, index) {
            index = skip_item(&tokens, index + 2);
            continue;
        }
        if let (TokenTree::Ident(name), Some(TokenTree::Punct(bang)), Some(TokenTree::Group(group))) =
            (&tokens[index], tokens.get(index + 1), tokens.get(index + 2))
            && bang.as_char() == '!'
            && let Some(kind) = Kind::of_macro(&name.to_string())
        {
            match Invocation::parse(kind, group.stream()) {
                Ok(invocation) => found.push(invocation),
                Err(error) => errors.push(format!("line {}: {error}", name.span().start().line)),
            }
            index += 3;
            continue;
        }
        if let TokenTree::Group(group) = &tokens[index] {
            invocations(group.stream(), found, errors);
        }
        index += 1;
    }
}

fn is_test_attribute(tokens: &[TokenTree], index: usize) -> bool {
    matches!(
        (&tokens[index], tokens.get(index + 1)),
        (TokenTree::Punct(hash), Some(TokenTree::Group(group)))
            if hash.as_char() == '#'
                && group.delimiter() == Delimiter::Bracket
                && group.stream().to_string().replace(' ', "") == "cfg(test)"
    )
}

fn skip_item(tokens: &[TokenTree], mut index: usize) -> usize {
    while let Some(token) = tokens.get(index) {
        index += 1;
        match token {
            TokenTree::Group(group) if group.delimiter() == Delimiter::Brace => break,
            TokenTree::Punct(punct) if punct.as_char() == ';' => break,
            _ => {}
        }
    }
    index
}

fn split_commas(input: TokenStream) -> Vec<TokenStream> {
    let mut parts = vec![Vec::new()];
    for token in input {
        match &token {
            TokenTree::Punct(punct) if punct.as_char() == ',' => parts.push(Vec::new()),
            _ => {
                if let Some(part) = parts.last_mut() {
                    part.push(token);
                }
            }
        }
    }
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .map(TokenStream::from_iter)
        .collect()
}

fn named(part: &TokenStream) -> Option<(String, TokenStream)> {
    let mut tokens = part.clone().into_iter();
    let (Some(TokenTree::Ident(name)), Some(TokenTree::Punct(equals))) =
        (tokens.next(), tokens.next())
    else {
        return None;
    };
    (equals.as_char() == '=').then(|| (name.to_string(), tokens.collect()))
}

fn literal(part: &TokenStream) -> Result<String, String> {
    syn::parse2::<syn::LitStr>(part.clone())
        .map(|literal| literal.value())
        .map_err(|_| format!("“{part}” is not a string literal"))
}
