#![no_main]

use std::fmt::Write;

use libfuzzer_sys::fuzz_target;
use web_css::{CssRule, parse_selector_list, parse_style_attribute, parse_stylesheet};

fn serialize(rules: &[CssRule], out: &mut String) {
    for rule in rules {
        match rule {
            CssRule::Style(style) => {
                for declaration in &style.declarations.declarations {
                    let _ = write!(out, "{}", declaration.value);
                }
                serialize(&style.rules, out);
            }
            CssRule::Media(_, rules)
            | CssRule::Supports(_, rules)
            | CssRule::Container(_, _, rules)
            | CssRule::Layer(_, rules) => serialize(rules, out),
            _ => {}
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let (css, _) = web_css::decode_stylesheet(data, None, None);
    let mut atoms = web_atoms::AtomTable::new();
    let sheet = parse_stylesheet(&css, &mut atoms);
    let mut out = String::new();
    serialize(&sheet.rules, &mut out);
    for declaration in parse_style_attribute(&css, &mut atoms).declarations {
        let _ = write!(out, "{}", declaration.value);
    }
    let _ = parse_selector_list(&css, &mut atoms, &sheet.namespaces);
});
