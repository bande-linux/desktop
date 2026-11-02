#![no_main]

use libfuzzer_sys::fuzz_target;
use web_atoms::AtomTable;
use web_css::selectors::{Namespaces, SelectorParser};
use web_css::{Parser, Tokenizer, preprocess};

fuzz_target!(|data: &[u8]| {
    let css = preprocess(&String::from_utf8_lossy(data)).into_owned();
    let mut atoms = AtomTable::new();
    let namespaces = Namespaces::default();
    let mut tokenizer = Tokenizer::new(&css);
    let mut input = Parser::new(&mut tokenizer);
    let Ok(list) = (SelectorParser {
        atoms: &mut atoms,
        namespaces: &namespaces,
        parent: None,
    })
    .parse_list(&mut input) else {
        return;
    };
    let mut tokenizer = Tokenizer::new(&css);
    let mut input = Parser::new(&mut tokenizer);
    let _ = SelectorParser {
        atoms: &mut atoms,
        namespaces: &namespaces,
        parent: Some(&list),
    }
    .parse_list(&mut input);
    let _ = list.max_specificity();
});
