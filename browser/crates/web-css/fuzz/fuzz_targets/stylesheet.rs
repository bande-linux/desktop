#![no_main]

use std::borrow::Cow;

use libfuzzer_sys::fuzz_target;
use web_css::{
    BlockContentsParser, ParseResult, Parser, RuleHandler, RuleListParser, Tokenizer, preprocess,
};

struct Walk;

fn walk(input: &mut Parser<'_, '_>) -> ParseResult<()> {
    while let Ok(token) = input.next_including_whitespace() {
        if token.block_end().is_some() {
            input.parse_nested_block(walk)?;
        }
    }
    Ok(())
}

impl<'i> RuleHandler<'i> for Walk {
    type AtPrelude = ();
    type QualifiedPrelude = ();
    type Item = ();

    fn at_rule_prelude(&mut self, _: Cow<'i, str>, input: &mut Parser<'i, '_>) -> ParseResult<()> {
        walk(input)
    }

    fn at_rule_block(&mut self, _: (), input: &mut Parser<'i, '_>) -> ParseResult<()> {
        BlockContentsParser::new(input, &mut Walk).for_each(drop);
        Ok(())
    }

    fn at_rule_without_block(&mut self, _: ()) -> ParseResult<()> {
        Ok(())
    }

    fn qualified_rule_prelude(&mut self, input: &mut Parser<'i, '_>) -> ParseResult<()> {
        walk(input)
    }

    fn qualified_rule_block(&mut self, _: (), input: &mut Parser<'i, '_>) -> ParseResult<()> {
        BlockContentsParser::new(input, &mut Walk).for_each(drop);
        Ok(())
    }

    fn declaration(&mut self, _: Cow<'i, str>, input: &mut Parser<'i, '_>) -> ParseResult<()> {
        walk(input)
    }
}

fuzz_target!(|data: &[u8]| {
    let css = preprocess(&String::from_utf8_lossy(data)).into_owned();
    let mut tokenizer = Tokenizer::with_unicode_ranges(&css);
    let mut input = Parser::new(&mut tokenizer);
    RuleListParser::new_for_stylesheet(&mut input, &mut Walk).for_each(drop);
    let mut tokenizer = Tokenizer::new(&css);
    let mut input = Parser::new(&mut tokenizer);
    let _ = web_css::nth::parse_nth(&mut input);
});
